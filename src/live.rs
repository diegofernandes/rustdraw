//! Ligação entre os assistentes de IA e o editor aberto.
//!
//! Com "Permitir edição por IA" ligado, o editor escuta em 127.0.0.1 (na porta
//! [`DEFAULT_PORT`], ou noutra se ela estiver ocupada) e atende de dois jeitos:
//!
//! - **MCP por HTTP** ("Streamable HTTP"): o assistente se conecta direto em
//!   `http://127.0.0.1:porta/mcp`, mandando `Authorization: Bearer <chave>`.
//! - **Ponte do `rustdraw mcp`**: o editor anota a porta e a chave em
//!   `mcp-live.json`, na pasta de dados do RustDraw; o `rustdraw mcp` lê esse
//!   arquivo e manda cada chamada de ferramenta por uma conexão com uma linha
//!   JSON de pedido e uma de resposta: `{"token", "client", "session", "name",
//!   "arguments"}` → `{"result": ...}` ou `{"error": "..."}`.
//!
//! Nos dois casos as ferramentas agem no desenho aberto (com desfazer). Só
//! conexões locais que conheçam a chave são atendidas, e o editor pede ao
//! usuário que aprove cada assistente antes de atender os pedidos dele.

use crate::model::new_id;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Tempo máximo que o editor pode levar para responder (ele pode estar no
/// meio de um arrasto ou de um texto, e o pedido espera terminar).
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// Tamanho máximo de um pedido (imagens vêm por caminho, não embutidas).
const MAX_REQUEST: u64 = 16 << 20;
/// Porta do endereço MCP do editor (fixa para a configuração do assistente
/// continuar valendo); se estiver ocupada, o editor usa outra qualquer.
pub const DEFAULT_PORT: u16 = 7470;

/// Endereço MCP (HTTP) do editor numa porta.
pub fn mcp_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// Chave nova para o endereço do editor.
pub fn new_token() -> String {
    format!("{}{}", new_id(), new_id())
}

pub fn discovery_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("rustdraw").join("mcp-live.json"))
}

/// Um pedido de ferramenta vindo do `rustdraw mcp`.
pub struct Request {
    /// Nome do assistente, como ele se apresentou.
    pub client: String,
    /// Um id por processo `rustdraw mcp`.
    pub session: String,
    pub name: String,
    pub args: Map<String, Value>,
    reply: Sender<Result<Value, String>>,
}

impl Request {
    pub fn respond(self, r: Result<Value, String>) {
        let _ = self.reply.send(r);
    }
}

/// Lado do editor: escuta enquanto existir.
pub struct Host {
    pub rx: Receiver<Request>,
    pub port: u16,
    stop: Arc<AtomicBool>,
    token: String,
    file: PathBuf,
}

impl Host {
    /// Começa a escutar em `port` (ou noutra, se ela estiver ocupada), aceitando
    /// a chave `token`. `wake` é chamado a cada pedido (para redesenhar o editor).
    pub fn start(
        port: u16,
        token: &str,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Host, String> {
        let file = discovery_path().ok_or("pasta de dados do usuário não encontrada")?;
        Self::start_at(&file, port, token, wake)
    }

    fn start_at(
        file: &Path,
        port: u16,
        token: &str,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Host, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| format!("escutar: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("escutar: {e}"))?;
        let token = token.to_string();
        let info = json!({"port": port, "token": token, "pid": std::process::id()});
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::edit::write_private(file, info.to_string().as_bytes())?;

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let editor = Editor {
            tx,
            wake: Arc::new(wake),
        };
        let sessions: Arc<HttpSessions> = Arc::default();
        let (stop2, token2) = (stop.clone(), token.clone());
        std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let (token, editor, sessions) =
                            (token2.clone(), editor.clone(), sessions.clone());
                        std::thread::spawn(move || serve(s, &token, &editor, &sessions));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        });
        Ok(Host {
            rx,
            port,
            stop,
            token,
            file: file.to_path_buf(),
        })
    }
}

/// Manda pedidos para o editor, de dentro dele (as conexões HTTP e a ponte).
#[derive(Clone)]
pub struct Editor {
    tx: Sender<Request>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Editor {
    /// Entrega a chamada ao editor e espera a resposta (o usuário pode estar
    /// aprovando o assistente, ou no meio de um arrasto).
    pub fn call(
        &self,
        client: &Client,
        name: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, CallError> {
        let (reply, wait) = mpsc::channel();
        let req = Request {
            client: short(&client.name, "assistente sem nome"),
            session: short(&client.session, "?"),
            name: name.to_string(),
            args: args.clone(),
            reply,
        };
        if self.tx.send(req).is_err() {
            return Err(CallError::Failed(
                "o editor parou de aceitar pedidos".into(),
            ));
        }
        (self.wake)();
        wait.recv_timeout(REPLY_TIMEOUT)
            .unwrap_or_else(|_| Err("o editor não respondeu a tempo".into()))
            .map_err(CallError::Failed)
    }
}

/// Nome curto para o painel; `default` se vier vazio.
fn short(v: &str, default: &str) -> String {
    let v = v.trim();
    if v.is_empty() { default } else { v }
        .chars()
        .take(80)
        .collect()
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // outra janela pode ter ligado depois; só apaga se o arquivo ainda é nosso
        let ours = std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| v["token"] == self.token.as_str());
        if ours {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

fn serve(s: TcpStream, token: &str, editor: &Editor, sessions: &HttpSessions) {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    let Ok(read) = s.try_clone() else { return };
    let mut r = BufReader::new(read.take(MAX_REQUEST));
    if r.read_line(&mut line).is_err() {
        return;
    }
    if line.trim_end().ends_with("HTTP/1.1") || line.trim_end().ends_with("HTTP/1.0") {
        return serve_http(&line, r, &s, token, editor, sessions);
    }
    let answer = |r: Result<Value, String>| {
        let v = match r {
            Ok(v) => json!({"result": v}),
            Err(e) => json!({"error": e}),
        };
        let mut s = &s;
        let _ = writeln!(s, "{v}");
    };
    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
        return answer(Err("pedido inválido".into()));
    };
    if msg["token"] != token {
        return answer(Err("chave inválida".into()));
    }
    let Some(name) = msg["name"].as_str() else {
        return answer(Err("falta `name`".into()));
    };
    let client = Client {
        name: msg["client"].as_str().unwrap_or("").to_string(),
        session: msg["session"].as_str().unwrap_or("").to_string(),
    };
    let args = msg["arguments"].as_object().cloned().unwrap_or_default();
    answer(editor.call(&client, name, &args).map_err(|e| match e {
        CallError::Failed(e) => e,
        CallError::NotRunning => "o editor não está aceitando pedidos".into(),
    }));
}

/// Sessões MCP abertas por HTTP, pelo `Mcp-Session-Id`.
type HttpSessions = Mutex<HashMap<String, crate::mcp::Server>>;

/// `localhost`, `127.0.0.1` ou `[::1]`, com ou sem porta.
fn is_local_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

/// Um pedido HTTP ao endereço MCP do editor ("Streamable HTTP", só com
/// respostas JSON). Cada conexão atende um pedido.
fn serve_http(
    first: &str,
    mut r: impl BufRead,
    out: &TcpStream,
    token: &str,
    editor: &Editor,
    sessions: &HttpSessions,
) {
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("").split('?').next().unwrap_or("");
    let mut headers = HashMap::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).unwrap_or(0) == 0 {
            return;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let header = |k: &str| headers.get(k).map(String::as_str);
    let reply = |status: &str, extra: &str, body: &str| {
        let mut out = out;
        let _ = write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        );
    };
    let fail = |status: &str, msg: &str| reply(status, "", &json!({"error": msg}).to_string());

    // páginas abertas no navegador não podem falar com o editor
    let origin_ok = header("origin").is_none_or(|o| {
        o.strip_prefix("http://")
            .or(o.strip_prefix("https://"))
            .is_some_and(is_local_host)
    });
    if !header("host").is_some_and(is_local_host) || !origin_ok {
        return fail("403 Forbidden", "só conexões locais");
    }
    if path != "/mcp" && path != "/" {
        return fail("404 Not Found", "o endereço MCP é /mcp");
    }
    if header("authorization").and_then(|a| a.strip_prefix("Bearer ")) != Some(token) {
        return fail(
            "401 Unauthorized",
            "chave inválida: use a do painel Edição por IA do RustDraw",
        );
    }
    let id = header("mcp-session-id").map(str::to_string);
    match method {
        "POST" => {}
        "DELETE" => {
            if let Some(id) = id {
                sessions.lock().unwrap().remove(&id);
            }
            return reply("200 OK", "", "");
        }
        _ => return reply("405 Method Not Allowed", "Allow: POST, DELETE\r\n", ""),
    }
    let Some(len) = header("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        return fail("411 Length Required", "falta Content-Length");
    };
    // antes de reservar a memória: um tamanho absurdo derrubaria o editor
    if len as u64 > MAX_REQUEST {
        return fail("413 Content Too Large", "pedido grande demais");
    }
    let mut body = vec![0; len];
    if r.read_exact(&mut body).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body);
    let initialize =
        serde_json::from_str::<Value>(&body).is_ok_and(|m| m["method"] == "initialize");
    let (id, mut server) = if initialize {
        let id = new_id();
        let client = Client {
            name: String::new(),
            session: id.clone(),
        };
        (id, crate::mcp::Server::for_editor(editor.clone(), client))
    } else {
        let Some(id) = id else {
            return fail("400 Bad Request", "falta o cabeçalho Mcp-Session-Id");
        };
        // a chamada pode demorar (aprovação); não segura as outras sessões
        let Some(server) = sessions.lock().unwrap().get(&id).cloned() else {
            return fail("404 Not Found", "sessão desconhecida; conecte de novo");
        };
        (id, server)
    };
    let resp = server.handle_line(&body);
    if initialize {
        sessions.lock().unwrap().insert(id.clone(), server);
    }
    let extra = format!("Mcp-Session-Id: {id}\r\n");
    match resp {
        Some(v) => reply("200 OK", &extra, &v.to_string()),
        None => reply("202 Accepted", &extra, ""),
    }
}

/// Quantos pedidos o histórico guarda.
const LOG_LEN: usize = 100;

/// Um processo `rustdraw mcp` que falou com o editor e a decisão do usuário.
pub struct Session {
    pub id: String,
    pub client: String,
    /// `None` enquanto o usuário não respondeu.
    pub allowed: Option<bool>,
}

/// Um pedido atendido (ou recusado), para o painel "Edição por IA".
pub struct LogEntry {
    /// Instante do editor (segundos), para mostrar "há 2 min".
    pub at: f64,
    pub client: String,
    pub tool: String,
    pub ok: bool,
    pub detail: String,
}

/// O que o editor sabe das conexões: quem pode editar e o que foi feito.
#[derive(Default)]
pub struct Sessions {
    pub list: Vec<Session>,
    pub log: std::collections::VecDeque<LogEntry>,
}

impl Sessions {
    /// Pode atender `req`? `None` = ainda esperando o usuário (um processo novo
    /// fica registrado como pendente). Assistentes em `trusted` entram direto.
    pub fn decide(&mut self, req: &Request, trusted: &[String]) -> Option<bool> {
        if let Some(s) = self.list.iter().find(|s| s.id == req.session) {
            return s.allowed;
        }
        let allowed = trusted.contains(&req.client).then_some(true);
        self.list.push(Session {
            id: req.session.clone(),
            client: req.client.clone(),
            allowed,
        });
        allowed
    }

    /// O primeiro processo esperando aprovação.
    pub fn pending(&self) -> Option<&Session> {
        self.list.iter().find(|s| s.allowed.is_none())
    }

    pub fn set(&mut self, id: &str, allowed: bool) {
        if let Some(s) = self.list.iter_mut().find(|s| s.id == id) {
            s.allowed = Some(allowed);
        }
    }

    pub fn record(&mut self, e: LogEntry) {
        self.log.push_front(e);
        self.log.truncate(LOG_LEN);
    }
}

/// Lado do `rustdraw mcp`: o editor não está aberto com a opção ligada, ou o
/// pedido falhou depois de chegar nele.
#[derive(Debug, PartialEq)]
pub enum CallError {
    NotRunning,
    Failed(String),
}

/// Quem está chamando: o assistente e o processo `rustdraw mcp`.
#[derive(Clone, Debug)]
pub struct Client {
    pub name: String,
    pub session: String,
}

/// Manda uma chamada de ferramenta para o editor aberto.
pub fn call(client: &Client, name: &str, args: &Map<String, Value>) -> Result<Value, CallError> {
    call_at(
        &discovery_path().ok_or(CallError::NotRunning)?,
        client,
        name,
        args,
    )
}

fn call_at(
    file: &Path,
    client: &Client,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Value, CallError> {
    let info: Value = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or(CallError::NotRunning)?;
    let port = info["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .ok_or(CallError::NotRunning)?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // arquivo esquecido por um editor que fechou de repente: ninguém escuta
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|_| CallError::NotRunning)?;
    let failed = |e: std::io::Error| CallError::Failed(format!("conexão com o editor: {e}"));
    s.set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(5)))
        .map_err(failed)?;
    let msg = json!({
        "token": info["token"],
        "client": client.name,
        "session": client.session,
        "name": name,
        "arguments": args,
    });
    writeln!(&s, "{msg}").map_err(failed)?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).map_err(failed)?;
    let v: Value = serde_json::from_str(&line)
        .map_err(|_| CallError::Failed("resposta inválida do editor".into()))?;
    match v.get("result") {
        Some(r) => Ok(r.clone()),
        None => Err(CallError::Failed(
            v["error"]
                .as_str()
                .unwrap_or("erro desconhecido")
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_cleanup() {
        let dir = std::env::temp_dir().join(format!("rustdraw-live-{}", new_id()));
        let file = dir.join("mcp-live.json");
        let args = Map::new();
        let me = Client {
            name: "Teste".into(),
            session: "s1".into(),
        };
        assert_eq!(call_at(&file, &me, "x", &args), Err(CallError::NotRunning));

        let host = Host::start_at(&file, 0, &new_token(), || {}).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "só o usuário lê a chave");
        }
        let client = {
            let (file, me) = (file.clone(), me.clone());
            std::thread::spawn(move || {
                let args = Map::new();
                (
                    call_at(&file, &me, "get_scene", &args),
                    call_at(&file, &me, "add_elements", &args),
                )
            })
        };
        let req = host.rx.recv().unwrap();
        assert_eq!(req.name, "get_scene");
        assert_eq!((req.client.as_str(), req.session.as_str()), ("Teste", "s1"));
        req.respond(Ok(json!({"ok": 1})));
        host.rx.recv().unwrap().respond(Err("não deu".into()));
        let (a, b) = client.join().unwrap();
        assert_eq!(a, Ok(json!({"ok": 1})));
        assert_eq!(b, Err(CallError::Failed("não deu".into())));

        // chave errada é recusada sem chegar no editor
        let mut info: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        info["token"] = json!("outra");
        let forged = dir.join("forged.json");
        std::fs::write(&forged, info.to_string()).unwrap();
        assert_eq!(
            call_at(&forged, &me, "get_scene", &args),
            Err(CallError::Failed("chave inválida".into()))
        );

        // aprovação: processo novo espera; assistente de confiança entra direto
        let (reply, _) = mpsc::channel();
        let req = |client: &str, session: &str| Request {
            client: client.into(),
            session: session.into(),
            name: "get_scene".into(),
            args: Map::new(),
            reply: reply.clone(),
        };
        let mut ss = Sessions::default();
        assert_eq!(ss.decide(&req("A", "1"), &[]), None);
        assert_eq!(ss.pending().map(|s| s.id.as_str()), Some("1"));
        ss.set("1", false);
        assert_eq!(ss.decide(&req("A", "1"), &[]), Some(false));
        assert_eq!(ss.decide(&req("B", "2"), &["B".into()]), Some(true));
        assert!(ss.pending().is_none());

        drop(host);
        assert!(!file.exists(), "fechar apaga o arquivo de descoberta");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Manda um pedido HTTP cru e devolve (status, cabeçalhos+corpo).
    fn http(port: u16, method: &str, headers: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(
            s,
            "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status = out[9..12].parse().unwrap();
        (status, out)
    }

    #[test]
    fn mcp_over_http() {
        let dir = std::env::temp_dir().join(format!("rustdraw-http-{}", new_id()));
        let host = Host::start_at(&dir.join("mcp-live.json"), 0, "segredo", || {}).unwrap();
        let port = host.port;
        let auth = "Authorization: Bearer segredo\r\n";
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"claude-code"}}}"#;

        assert_eq!(http(port, "POST", "", init).0, 401);
        assert_eq!(
            http(port, "POST", "Authorization: Bearer x\r\n", init).0,
            401
        );
        let evil = format!("{auth}Origin: http://exemplo.com\r\n");
        assert_eq!(http(port, "POST", &evil, init).0, 403);
        assert_eq!(http(port, "GET", auth, "").0, 405);
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        assert_eq!(http(port, "POST", auth, list).0, 400, "sem sessão");

        let (status, out) = http(port, "POST", auth, init);
        assert_eq!(status, 200);
        assert!(out.contains("\"serverInfo\""));
        let session = out
            .lines()
            .find_map(|l| l.strip_prefix("Mcp-Session-Id: "))
            .unwrap()
            .to_string();
        let with = format!("{auth}Mcp-Session-Id: {session}\r\n");
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert_eq!(http(port, "POST", &with, note).0, 202);
        let (status, out) = http(port, "POST", &with, list);
        assert_eq!(status, 200);
        assert!(out.contains("add_elements"));
        assert!(!out.contains("\"file\""), "no editor não há `file`");

        // `file` passaria por fora da aprovação: é recusado sem chegar ao editor
        let target = dir.join("outro.excalidraw");
        let body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
            "name": "new_scene", "arguments": {"file": target, "overwrite": true}}});
        let (status, out) = http(port, "POST", &with, &body.to_string());
        assert_eq!(status, 200);
        assert!(out.contains("\"isError\":true"), "{out}");
        assert!(!target.exists());
        assert!(host.rx.try_recv().is_err());

        // tamanho absurdo é recusado antes de reservar a memória
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(
            s,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{with}\
             Content-Length: 1000000000000\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 413"), "{out}");

        // chamada sem `file`: vai para o editor, com o nome do assistente e a sessão
        let call = std::thread::spawn(move || {
            let body = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_scene","arguments":{}}}"#;
            http(port, "POST", &with, body)
        });
        let req = host.rx.recv().unwrap();
        assert_eq!(req.name, "get_scene");
        assert_eq!(req.client, "claude-code");
        assert_eq!(req.session, session);
        req.respond(Ok(json!({"content": [{"type": "text", "text": "vazio"}]})));
        let (status, out) = call.join().unwrap();
        assert_eq!(status, 200);
        assert!(out.contains("vazio"));

        let gone = "Authorization: Bearer segredo\r\nMcp-Session-Id: outra\r\n";
        assert_eq!(http(port, "POST", gone, list).0, 404);
        drop(host);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

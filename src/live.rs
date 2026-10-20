//! Ligação entre o `rustdraw mcp` e o editor aberto.
//!
//! Com "Permitir edição por IA" ligado, o editor escuta em 127.0.0.1 numa porta
//! qualquer e anota a porta e uma chave aleatória em `mcp-live.json`, na pasta
//! de dados do RustDraw. O `rustdraw mcp` lê esse arquivo e manda cada chamada
//! de ferramenta para o editor, que a aplica no desenho aberto (com desfazer).
//! Só aceita conexões locais que conheçam a chave (nova a cada vez que a opção
//! é ligada); nada sai da máquina. Além disso, o editor pede ao usuário que
//! aprove cada assistente antes de atender os pedidos dele.
//!
//! Cada conexão leva um pedido e uma resposta, uma linha JSON cada:
//! `{"token", "client", "session", "name", "arguments"}` → `{"result": ...}`
//! ou `{"error": "..."}`. `client` é o nome do assistente (do `initialize` do
//! MCP) e `session` identifica um processo `rustdraw mcp`.

use crate::model::new_id;
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// Tempo máximo que o editor pode levar para responder (ele pode estar no
/// meio de um arrasto ou de um texto, e o pedido espera terminar).
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// Tamanho máximo de um pedido (imagens vêm por caminho, não embutidas).
const MAX_REQUEST: u64 = 16 << 20;

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
    /// Começa a escutar. `wake` é chamado a cada pedido (para redesenhar o editor).
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Result<Host, String> {
        let file = discovery_path().ok_or("pasta de dados do usuário não encontrada")?;
        Self::start_at(&file, wake)
    }

    fn start_at(file: &Path, wake: impl Fn() + Send + Sync + 'static) -> Result<Host, String> {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("escutar: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("escutar: {e}"))?;
        let token = format!("{}{}", new_id(), new_id());
        let info = json!({"port": port, "token": token, "pid": std::process::id()});
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(file, info.to_string()).map_err(|e| format!("{}: {e}", file.display()))?;

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, token2, wake) = (stop.clone(), token.clone(), Arc::new(wake));
        std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let (tx, token, wake) = (tx.clone(), token2.clone(), wake.clone());
                        std::thread::spawn(move || serve(s, &token, &tx, &*wake));
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

fn serve(s: TcpStream, token: &str, tx: &Sender<Request>, wake: &(dyn Fn() + Send + Sync)) {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    let Ok(read) = s.try_clone() else { return };
    if BufReader::new(read.take(MAX_REQUEST))
        .read_line(&mut line)
        .is_err()
    {
        return;
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
    let (reply, wait) = mpsc::channel();
    let text = |k: &str, default: &str| {
        let v = msg[k].as_str().unwrap_or("").trim();
        if v.is_empty() { default } else { v }
            .chars()
            .take(80)
            .collect::<String>()
    };
    let req = Request {
        client: text("client", "assistente sem nome"),
        session: text("session", "?"),
        name: name.to_string(),
        args: msg["arguments"].as_object().cloned().unwrap_or_default(),
        reply,
    };
    if tx.send(req).is_err() {
        return answer(Err("o editor parou de aceitar pedidos".into()));
    }
    wake();
    answer(
        wait.recv_timeout(REPLY_TIMEOUT)
            .unwrap_or_else(|_| Err("o editor não respondeu a tempo".into())),
    );
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

        let host = Host::start_at(&file, || {}).unwrap();
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
}

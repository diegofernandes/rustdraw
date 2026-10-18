//! Percebe quando o arquivo aberto é alterado por outro programa (o `rustdraw mcp`,
//! um editor de texto, o git...).
//!
//! Sem dependências: consulta a data de modificação e o tamanho do arquivo, e só
//! lê o conteúdo quando eles mudam. O conteúdo é comparado por hash com o último
//! que o editor conhece, então as gravações do próprio editor não contam.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Watch {
    pub path: PathBuf,
    /// `None` força a leitura na próxima consulta.
    stamp: Option<(SystemTime, u64)>,
    hash: u64,
}

fn hash(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

impl Watch {
    /// `known` é o conteúdo que o editor tem do arquivo (o que leu ou gravou).
    ///
    /// A data de modificação não é guardada aqui: se outro programa gravar entre
    /// a nossa gravação e esta chamada, a primeira consulta ainda percebe.
    pub fn new(path: &Path, known: &[u8]) -> Self {
        Self {
            path: path.to_path_buf(),
            stamp: None,
            hash: hash(known),
        }
    }

    /// Devolve o novo conteúdo quando o arquivo mudou para algo diferente do conhecido.
    ///
    /// Um arquivo sumido ou ilegível é ignorado: pode ser uma gravação em andamento.
    pub fn poll(&mut self) -> Option<String> {
        let meta = std::fs::metadata(&self.path).ok()?;
        let stamp = (meta.modified().ok()?, meta.len());
        if self.stamp == Some(stamp) {
            return None;
        }
        let bytes = std::fs::read(&self.path).ok()?;
        self.stamp = Some(stamp);
        let h = hash(&bytes);
        if h == self.hash {
            return None;
        }
        self.hash = h;
        String::from_utf8(bytes).ok()
    }

    /// Esquece o conteúdo lido (que não deu para abrir), para tentar de novo depois.
    pub fn retry(&mut self) {
        self.stamp = None;
        self.hash = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rustdraw-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn ignores_own_contents_and_sees_other_writes() {
        let p = tmp("a.excalidraw");
        std::fs::write(&p, "um").unwrap();
        let mut w = Watch::new(&p, b"um");
        assert_eq!(w.poll(), None);
        assert_eq!(w.poll(), None);

        std::fs::write(&p, "dois, mais longo").unwrap();
        assert_eq!(w.poll().as_deref(), Some("dois, mais longo"));
        assert_eq!(w.poll(), None);

        // o editor grava: a nova referência é o que ele gravou
        std::fs::write(&p, "três").unwrap();
        let mut w = Watch::new(&p, b"tr\xc3\xaas");
        assert_eq!(w.poll(), None);
    }

    #[test]
    fn sees_change_made_right_after_own_save() {
        let p = tmp("b.excalidraw");
        std::fs::write(&p, "meu").unwrap();
        // outro programa grava antes da primeira consulta
        std::fs::write(&p, "deles").unwrap();
        let mut w = Watch::new(&p, b"meu");
        assert_eq!(w.poll().as_deref(), Some("deles"));
    }

    #[test]
    fn missing_file_and_retry() {
        let p = tmp("c.excalidraw");
        let _ = std::fs::remove_file(&p);
        let mut w = Watch::new(&p, b"x");
        assert_eq!(w.poll(), None);
        std::fs::write(&p, "{meio").unwrap();
        assert_eq!(w.poll().as_deref(), Some("{meio"));
        w.retry();
        assert_eq!(w.poll().as_deref(), Some("{meio"));
    }
}

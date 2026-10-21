//! Abrir desenhos pelo Finder no macOS (duplo clique, "Abrir com", soltar no
//! ícone do Dock). O Finder não passa o arquivo na linha de comando: ele manda
//! o Apple Event "abrir documentos", que chega aqui e entra numa fila que o
//! editor esvazia a cada quadro.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{ClassType, define_class, msg_send, sel};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString,
};

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}
const CORE_EVENT_CLASS: u32 = four_cc(b"aevt");
const OPEN_DOCUMENTS: u32 = four_cc(b"odoc");
const DIRECT_OBJECT: u32 = four_cc(b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "RustDrawOpenHandler"]
    struct OpenHandler;

    unsafe impl NSObjectProtocol for OpenHandler {}

    impl OpenHandler {
        /// O NSApplication instala os tratadores padrão antes deste aviso e
        /// só entrega os pedidos de abertura depois dele.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _note: &NSNotification) {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            // SAFETY: o objeto vive até o fim do processo (ver `install`) e o
            // seletor é o método abaixo, com a assinatura que o
            // NSAppleEventManager espera.
            unsafe {
                let _: () = msg_send![
                    &*manager,
                    setEventHandler: self,
                    andSelector: sel!(openDocuments:withReply:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS
                ];
            }
        }

        #[unsafe(method(openDocuments:withReply:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            // SAFETY: paramDescriptorForKeyword: recebe um AEKeyword (u32).
            let list: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
            let Some(list) = list else { return };
            let items: Vec<_> = match list.numberOfItems() {
                0 => vec![list],
                n => (1..=n).filter_map(|i| list.descriptorAtIndex(i)).collect(),
            };
            let paths = items
                .iter()
                .filter_map(|d| d.fileURLValue()?.path())
                .map(|p| PathBuf::from(p.to_string()));
            OPENED.lock().unwrap_or_else(|e| e.into_inner()).extend(paths);
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        }
    }
);

/// Passa a receber os pedidos de abertura. Chamar antes de `run_native`.
pub fn install() {
    // SAFETY: NSObject sem estado; `new` devolve o objeto já retido.
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::class(), new] };
    let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
    // SAFETY: o seletor existe em OpenHandler e recebe a NSNotification.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &handler,
            sel!(willFinishLaunching:),
            Some(&name),
            None,
        );
    }
    // nem a central de avisos nem o NSAppleEventManager retêm o objeto
    std::mem::forget(handler);
}

/// Contexto usado para acordar a janela quando chega um arquivo.
pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// Arquivos pedidos desde a última chamada.
pub fn take_opened() -> Vec<PathBuf> {
    std::mem::take(&mut *OPENED.lock().unwrap_or_else(|e| e.into_inner()))
}

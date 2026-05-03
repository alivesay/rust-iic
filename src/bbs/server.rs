use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::bbs::session;
use crate::bbs::source::{FileKind, SourceRef};
use crate::bbs::sources;

#[derive(Debug, Clone)]
pub enum BbsEvent {
    DownloadCompleted {
        name: String,
        path: PathBuf,

        #[allow(dead_code)]
        kind: FileKind,
    },

    #[allow(dead_code)]
    DownloadFailed {
        name: String,
        error: String,
    },
    Disconnected,
}

pub struct BbsHandle {
    pub addr: SocketAddr,
    pub events: mpsc::Receiver<BbsEvent>,

    _shutdown_tx: mpsc::Sender<()>,
    _thread: JoinHandle<()>,
}

pub fn start() -> io::Result<BbsHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;

    let (events_tx, events_rx) = mpsc::channel::<BbsEvent>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    let sources = sources::all();

    let thread = thread::spawn(move || accept_loop(listener, events_tx, shutdown_rx, sources));

    log::info!("bbs: listening on {}", addr);

    Ok(BbsHandle {
        addr,
        events: events_rx,
        _shutdown_tx: shutdown_tx,
        _thread: thread,
    })
}

fn accept_loop(
    listener: TcpListener,
    events: mpsc::Sender<BbsEvent>,
    shutdown: mpsc::Receiver<()>,
    sources: Vec<SourceRef>,
) {
    loop {
        if shutdown.try_recv().is_ok() {
            log::info!("bbs: shutting down");
            return;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                log::info!("bbs: accepted {}", peer);
                let events = events.clone();
                let sources = sources.clone();
                thread::spawn(move || {
                    if let Err(e) = session::run(
                        stream.try_clone().unwrap_or(stream),
                        sources,
                        events.clone(),
                    ) {
                        log::warn!("bbs: session ended: {}", e);
                    }
                    let _ = events.send(BbsEvent::Disconnected);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                log::warn!("bbs: accept error: {}", e);
                return;
            }
        }
    }
}

#[allow(dead_code)]
fn _shutdown_uses(stream: &std::net::TcpStream) -> io::Result<()> {
    stream.shutdown(Shutdown::Both)
}

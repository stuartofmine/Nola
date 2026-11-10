// SPDX-License-Identifier: Apache-2.0
//! Coalesced frame requests. Producers never sleep to animate terminal output.
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

static REQUESTS: OnceLock<mpsc::SyncSender<()>> = OnceLock::new();

pub(crate) fn request() {
    let sender = REQUESTS.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("nola-terminal-frames".into()).spawn(move || {
            while receiver.recv().is_ok() {
                // A bounded mailbox coalesces all mutations in this frame.
                std::thread::sleep(Duration::from_millis(16));
                while receiver.try_recv().is_ok() {}
                crate::terminal_ui::paint_pending_frame();
            }
        }).expect("start terminal frame scheduler");
        sender
    });
    let _ = sender.try_send(());
}

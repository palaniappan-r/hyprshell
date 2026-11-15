use anyhow::{Context, bail};
use async_channel::Sender;
use core_lib::transfer;
use core_lib::transfer::{ExternalTransferType, SWITCH_EVENT_PREFIX};
use core_lib::util::get_daemon_socket_path_buff;
use std::fs::remove_file;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net;
use std::os::unix::net::UnixStream;
use tracing::{debug, debug_span, instrument, warn};

/// Subscribe before installing bindings, so no key event can be lost between
/// binding installation and listener startup. One stream preserves key order.
pub fn start_switch_event_listener(
    event_sender: Sender<ExternalTransferType>,
) -> anyhow::Result<()> {
    let path = get_daemon_socket_path_buff().with_file_name(".socket2.sock");
    let stream = UnixStream::connect(&path)
        .with_context(|| format!("Cannot subscribe to compositor events: {}", path.display()))?;
    std::thread::spawn(move || {
        if let Err(error) = forward_switch_events(BufReader::new(stream), &event_sender) {
            warn!("Switcher event stream disconnected: {error}");
        }
    });
    Ok(())
}

fn forward_switch_events(
    reader: impl BufRead,
    sender: &Sender<ExternalTransferType>,
) -> anyhow::Result<()> {
    for line in reader.lines() {
        match switch_event(&line?) {
            Ok(Some(transfer)) => sender.send_blocking(transfer)?,
            Ok(None) => {}
            Err(error) => warn!("Invalid switcher event: {error}"),
        }
    }
    Ok(())
}

fn switch_event(line: &str) -> anyhow::Result<Option<ExternalTransferType>> {
    let Some(payload) = line
        .strip_prefix("custom>>")
        .and_then(|payload| payload.strip_prefix(SWITCH_EVENT_PREFIX))
    else {
        return Ok(None);
    };
    let transfer = transfer::receive_from_buffer(payload.as_bytes().to_vec())?;
    if matches!(
        transfer,
        ExternalTransferType::OpenSwitch(_) | ExternalTransferType::CloseSwitch(_)
    ) {
        Ok(Some(transfer))
    } else {
        Ok(None)
    }
}

pub fn socket_handler(event_sender: &Sender<ExternalTransferType>) {
    let _span = debug_span!("socket_handler").entered();
    let path = &get_daemon_socket_path_buff();
    let listener = {
        // remove old PATH
        if path.exists() {
            remove_file(path).expect("Unable to remove old socket file");
        }
        net::UnixListener::bind(path)
            .unwrap_or_else(|_| panic!("Failed to bind to socket {}", path.display()))
    };
    debug!("Starting socket on {}", path.display());

    loop {
        let path = listener.accept();
        match path {
            Ok((conn, _)) => {
                handle_client(conn, event_sender)
                    .context("Failed to handle client")
                    .unwrap_or_else(|e| {
                        warn!("Failed to handle connection: {e:?}");
                    });
            }
            Err(e) => {
                warn!("Failed to accept connection: {e:?}");
            }
        }
    }
}

#[instrument(level = "debug", skip(stream, event_sender))]
fn handle_client(
    mut stream: UnixStream,
    event_sender: &Sender<ExternalTransferType>,
) -> anyhow::Result<()> {
    let mut buffer = vec![];
    let mut reader = BufReader::new(&mut stream);
    reader
        .read_until(b'\0', &mut buffer)
        .context("Can't read data from socket")?;
    if buffer.is_empty() {
        return Ok(());
    }
    let transfer = match transfer::receive_from_buffer(buffer) {
        Ok(transfer) => transfer,
        Err(err) => {
            let _ = stream
                .write_all(b"ERR")
                .and_then(|()| stream.write_all(b"\0"));
            bail!("Invalid transfer received.\n{err:?}");
        }
    };
    event_sender
        .send_blocking(transfer)
        .context("Failed to send transfer")?;
    let _ = stream
        .write_all(b"OK")
        .and_then(|()| stream.write_all(b"\0"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{forward_switch_events, switch_event};
    use core_lib::transfer::{CloseSwitch, ExternalTransferType, OpenSwitch};
    use std::io::{BufReader, Cursor};

    #[test]
    fn rapid_press_release_events_stay_ordered_across_fragmented_reads() {
        let mut stream = String::new();
        let mut expected = Vec::new();
        for reverse in [false, true].into_iter().cycle().take(100) {
            stream.push_str("activewindowv2>>1234\n");
            stream.push_str(&format!(
                "custom>>hyprshell-switch:{{\"OpenSwitch\":{{\"reverse\":{reverse}}}}}\n"
            ));
            stream.push_str("custom>>hyprshell-switch:{\"CloseSwitch\":{\"switch\":true}}\n");
            expected.push(ExternalTransferType::OpenSwitch(OpenSwitch { reverse }));
            expected.push(ExternalTransferType::CloseSwitch(CloseSwitch {
                switch: true,
            }));
        }
        let (send, receive) = async_channel::unbounded();
        forward_switch_events(BufReader::with_capacity(7, Cursor::new(stream)), &send)
            .expect("forward compositor events");
        let received: Vec<_> = std::iter::from_fn(|| receive.try_recv().ok()).collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn unrelated_and_malformed_events_do_not_create_switch_sessions() {
        for line in [
            "activewindowv2>>1234",
            "custom>>other-app:event",
            "custom>>hyprshell-switch:\"Reload\"",
        ] {
            assert_eq!(switch_event(line).expect("parse event"), None);
        }
        assert!(switch_event("custom>>hyprshell-switch:invalid-json").is_err());
    }
}

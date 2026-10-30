//! The agent the launcher's requests go through (`agent`): kept between them, with a connection
//! kept open for each file fetched at a time, so Minecraft's 5,000 asset files reuse a handful of
//! connections instead of opening (and looking the host up for) one each.
use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex};

/// A local HTTP server that answers `ok` to every request on connections it keeps open, and how
/// many connections it accepted. Its answers wait until requests have come in `together` at a
/// time, so that `together` requests at once need a connection each.
fn server(together: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counted = accepted.clone();
    let arrived = Arc::new((Mutex::new(0usize), Condvar::new()));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            counted.fetch_add(1, SeqCst);
            let arrived = arrived.clone();
            std::thread::spawn(move || answer(stream, together, &arrived));
        }
    });
    (url, accepted)
}

/// Answers each request on `stream` once its group of `together` requests has arrived.
fn answer(stream: TcpStream, together: usize, arrived: &(Mutex<usize>, Condvar)) {
    let (count, ready) = arrived;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        // The request's head, up to its empty line; none once the client closed the connection.
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) if line == "\r\n" => break,
                Ok(_) => {}
            }
        }
        let mut n = count.lock().unwrap();
        *n += 1;
        let group = n.div_ceil(together) * together;
        ready.notify_all();
        drop(ready.wait_while(n, |n| *n < group).unwrap());
        let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        if reader.get_mut().write_all(reply).is_err() {
            return;
        }
    }
}

#[test]
fn files_fetched_one_after_another_share_one_connection() {
    let (url, accepted) = server(1);
    for _ in 0..3 {
        assert_eq!(get_bytes(&url).unwrap(), b"ok");
    }
    assert_eq!(accepted.load(SeqCst), 1);
}

/// Minecraft's assets are fetched 8 at a time (`download_for`): each of the 8 keeps its connection
/// for its next file.
#[test]
fn files_fetched_eight_at_a_time_keep_their_eight_connections() {
    let (url, accepted) = server(8);
    for _ in 0..2 {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| assert_eq!(get_bytes(&url).unwrap(), b"ok"));
            }
        });
    }
    assert_eq!(accepted.load(SeqCst), 8);
}

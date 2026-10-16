use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};

fn server(handler: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/player", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let thread = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "client did not connect"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 8192);
        }
        handler(stream);
    });
    (url, thread)
}

fn fetch_with_limit(url: String, limit: usize) -> Result<Vec<u8>, String> {
    let (send, receive) = mpsc::channel();
    let task = spawn(async move {
        send.send(platform::fetch(&url, limit).await).unwrap();
    })
    .unwrap();
    let result = receive.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(task);
    result
}

fn response(wire: &'static [u8], limit: usize) -> Result<Vec<u8>, String> {
    let (url, thread) = server(move |mut stream| {
        let _ = stream.write_all(wire);
    });
    let result = fetch_with_limit(url, limit);
    thread.join().unwrap();
    result
}

#[test]
fn http_reads_accept_small_exact_and_unframed_bodies() {
    assert_eq!(
        response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            8
        )
        .unwrap(),
        b"{}"
    );
    assert_eq!(
        response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n{\"x\":10}",
            8
        )
        .unwrap(),
        b"{\"x\":10}"
    );
    assert_eq!(
        response(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}", 8).unwrap(),
        b"{}"
    );
}

#[test]
fn http_limits_do_not_trust_response_length_headers() {
    assert!(response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n",
        8
    )
    .unwrap_err()
    .contains("limit"));
    assert!(response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n{}",
        8
    )
    .is_err());
    assert!(response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\nConnection: close\r\n\r\n9\r\n123456789\r\n0\r\n\r\n", 8).is_err());
    assert!(response(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        8
    )
    .unwrap_err()
    .contains("404"));
}

#[test]
fn oversized_stream_is_closed_before_the_server_finishes_it() {
    let (url, thread) = server(move |mut stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\n123456789\r\n")
            .unwrap();
        stream.flush().unwrap();
        // No terminating chunk: waiting for the full response would hang.
        let result = stream.read(&mut [0; 1]);
        assert!(
            matches!(result, Ok(0))
                || result.as_ref().is_err_and(|error| matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )),
            "stream did not close: {result:?}"
        );
    });
    assert!(fetch_with_limit(url, 8).unwrap_err().contains("limit"));
    thread.join().unwrap();
}

#[test]
fn cancelling_a_stalled_transfer_closes_it_and_unblocks_the_next_import() {
    let (started, wait_started) = mpsc::channel();
    let (first_url, first_server) = server(move |mut stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
        started.send(()).unwrap();
        let result = stream.read(&mut [0; 1]);
        assert!(
            matches!(result, Ok(0))
                || result.as_ref().is_err_and(|error| matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )),
            "stream did not close: {result:?}"
        );
    });
    let (first_done, first_result) = mpsc::channel();
    let first = fetch(first_url, move |result| {
        first_done.send(result).unwrap();
    })
    .unwrap();
    wait_started.recv_timeout(Duration::from_secs(10)).unwrap();
    let (second_started, second_seen) = mpsc::channel();
    let (second_url, second_server) = server(move |mut stream| {
        second_started.send(()).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    let (second_done, second_result) = mpsc::channel();
    let second = fetch(second_url, move |result| {
        second_done.send(result).unwrap();
    })
    .unwrap();
    assert!(matches!(
        second_seen.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(first);
    assert_eq!(
        second_result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap(),
        "{}"
    );
    assert!(first_result.recv_timeout(Duration::from_secs(1)).is_err());
    drop(second);
    first_server.join().unwrap();
    second_server.join().unwrap();
}

#[test]
fn cancelling_a_queued_import_never_starts_its_transfer() {
    let (ready, started) = mpsc::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let blocker = spawn(async move {
        let _permit = transfers().acquire().await;
        ready.send(()).unwrap();
        released.await.unwrap();
    })
    .unwrap();
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done, received) = mpsc::channel();
    let queued = fetch("http://127.0.0.1:1/unused".into(), move |result| {
        done.send(result).unwrap();
    })
    .unwrap();
    thread::sleep(Duration::from_millis(30));
    drop(queued);
    release.send(()).unwrap();
    assert!(received.recv_timeout(Duration::from_secs(5)).is_err());
    drop(blocker);
}

#[test]
fn local_file_limits_are_checked_before_buffering() {
    let directory = tempfile::tempdir().unwrap();
    let small = directory.path().join("small.json");
    let large = directory.path().join("large.json");
    std::fs::write(&small, b"{}").unwrap();
    std::fs::File::create(&large)
        .unwrap()
        .set_len(MAX_IMPORT_BYTES as u64 + 1)
        .unwrap();
    let (send, receive) = mpsc::channel();
    let task = spawn(async move {
        assert_eq!(platform::read_file(&small, 8).await.unwrap(), b"{}");
        assert!(platform::read_file(&large, MAX_IMPORT_BYTES)
            .await
            .unwrap_err()
            .contains("limit"));
        send.send(()).unwrap();
    })
    .unwrap();
    receive.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(task);
}

#[test]
fn bounded_accumulation_stops_at_one_byte_past_the_limit() {
    let mut body = Body::new(8).unwrap();
    body.push(b"12345678").unwrap();
    assert!(body.push(&[0; 1024]).is_err());
    assert_eq!(body.bytes.len(), 9);
    let bytes = b"{\"userId\":18446744073709551615}".to_vec();
    let pointer = bytes.as_ptr();
    let text = decode_bytes(bytes).unwrap();
    assert_eq!(text.as_ptr(), pointer);
    assert!(text.contains("18446744073709551615"));
    assert_eq!(decode_bytes(b"\xef\xbb\xbf{}".to_vec()).unwrap(), "{}");
}

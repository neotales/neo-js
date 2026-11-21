//! Integration tests for `neo:fetch` through the public `NeoJs` API.
//!
//! Every test that needs a server starts an isolated loopback listener on an
//! ephemeral port. No test uses an external network. The server scripts a
//! fixed sequence of responses, which covers success, progress, status errors,
//! redirects, truncation, and oversize bodies.

use std::{
    io::{BufRead, BufReader, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use neojs_core::{NeoJs, Permissions};

fn runtime(network: bool, filesystem: bool) -> NeoJs {
    NeoJs::new(Permissions {
        network,
        filesystem,
        ..Permissions::default()
    })
    .unwrap()
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

fn path_string(path: &Path) -> String {
    js_string(path.to_str().unwrap())
}

fn assert_no_temporary_files(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.starts_with(".neo-download-"),
            "temporary file left behind: {name}"
        );
    }
}

fn assert_no_authorization(requests: &[RecordedRequest]) {
    for request in requests {
        for header in &request.headers {
            assert!(
                !header.to_ascii_lowercase().starts_with("authorization:"),
                "unexpected Authorization header: {header}"
            );
        }
    }
}

fn assert_error_code(js: &NeoJs, script: &str, expected: &str) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let name = format!("fetch-test-{}.js", COUNTER.fetch_add(1, Ordering::Relaxed));
    js.run_module(&name, script).unwrap();
    assert_eq!(
        js.eval("globalThis.lastCode").unwrap(),
        expected,
        "unexpected error code"
    );
}

#[test]
fn download_inside_tokio_runtime() {
    let server = TestServer::start(vec![Response::Ok(b"inside".to_vec())]);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("inside");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let js = crate::runtime(true, true);
        let script = format!(
            "import {{ download }} from 'neo:fetch'; download({}, {});",
            js_string(&server.url("/")),
            path_string(&destination)
        );
        js.run_module("tokio-download.js", &script).unwrap();
    });
    assert_eq!(std::fs::read(destination).unwrap(), b"inside");
}

#[test]
fn downloads_from_concurrent_tokio_hosts_are_independent() {
    let server = TestServer::start(
        (0..8)
            .map(|_| Response::Ok(b"concurrent".to_vec()))
            .collect(),
    );
    let directory = tempfile::tempdir().unwrap();
    let mut threads = Vec::new();
    for index in 0..8 {
        let url = server.url("/");
        let destination = directory.path().join(format!("download-{index}"));
        threads.push(thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
            runtime.block_on(async {
                let js = crate::runtime(true, true);
                let script = format!(
                    "import {{ download }} from 'neo:fetch'; download({}, {}, {{progress() {{}}}});",
                    js_string(&url),
                    path_string(&destination)
                );
                js.run_module("concurrent-download.js", &script).unwrap();
            });
            assert_eq!(std::fs::read(destination).unwrap(), b"concurrent");
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(server.request_count(), 8);
}

#[test]
fn tokio_host_callback_failure_and_deadline_cancel_before_return() {
    let server = TestServer::start(vec![Response::Ok(vec![1]), Response::Ok(vec![2])]);
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("callback");
    let second = directory.path().join("timeout");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let js = crate::runtime(true, true);
        let script = format!(
            "import {{ download }} from 'neo:fetch';
             globalThis.calls = 0; globalThis.codes = [];
             try {{ download({url}, {first}, {{progress() {{ calls++; throw Error('stop'); }}}}); }}
             catch (e) {{ codes.push(e.code); }}
             try {{ download({url}, {second}, {{timeoutMs: 250, progress() {{
                 calls++; const until = Date.now() + 350; while (Date.now() < until) {{}}
             }}}}); }} catch (e) {{ codes.push(e.code); }}",
            url = js_string(&server.url("/")),
            first = path_string(&first),
            second = path_string(&second),
        );
        js.run_module("tokio-cancel.js", &script).unwrap();
        assert_eq!(
            js.eval("codes.join(',')").unwrap(),
            "FETCH_CALLBACK,FETCH_TIMEOUT"
        );
        assert_eq!(js.eval("String(calls)").unwrap(), "2");
        thread::sleep(Duration::from_millis(50));
        assert_eq!(js.eval("String(calls)").unwrap(), "2");
    });
    assert!(!first.exists());
    assert!(!second.exists());
    assert_no_temporary_files(directory.path());
}

#[test]
fn unsolicited_compressed_bodies_are_saved_and_bounded_as_wire_bytes() {
    let gzip = vec![
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 203, 72, 205, 201, 201, 87, 200, 64, 39, 1, 227, 81, 61,
        141, 23, 0, 0, 0,
    ];
    let mut br = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut br, 4096, 5, 22);
        encoder.write_all(b"hello hello hello hello").unwrap();
    }
    for (encoding, body) in [("gzip", gzip), ("br", br)] {
        let response = || {
            let mut bytes = format!("HTTP/1.1 200 OK\r\nContent-Encoding: {encoding}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
            bytes.extend_from_slice(&body);
            Response::Raw(bytes)
        };
        let server = TestServer::start(vec![response(), response()]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("data");
        let js = runtime(true, true);
        let script = format!(
            "import {{ download }} from 'neo:fetch'; let seen = []; let r = download({}, {}, {{maxBytes: {}, progress: (n,t) => seen.push([n,t])}}); if (r.bytes !== {} || seen[0][1] !== {} || seen.at(-1)[0] !== {}) throw Error('wire progress');",
            js_string(&server.url("/")),
            path_string(&dest),
            body.len(),
            body.len(),
            body.len(),
            body.len()
        );
        js.run_module(&format!("encoded-{encoding}.js"), &script)
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let script = format!(
            "import {{ download }} from 'neo:fetch'; try {{ download({}, {}, {{maxBytes: {}}}); }} catch (e) {{ globalThis.lastCode = e.code; }}",
            js_string(&server.url("/")),
            path_string(&dir.path().join("limited")),
            body.len() - 1
        );
        assert_error_code(&js, &script, "FETCH_LIMIT");
    }
}

enum Response {
    Ok(Vec<u8>),
    Status(u16, Vec<u8>),
    Redirect(String),
    Raw(Vec<u8>),
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    line: String,
    headers: Vec<String>,
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(responses: Vec<Response>) -> Self {
        Self::start_with(|_| responses)
    }

    fn start_with<F>(responses: F) -> Self
    where
        F: FnOnce(SocketAddr) -> Vec<Response>,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let responses = responses(addr);
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop_for_thread = Arc::clone(&stop);
        let requests_for_thread = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            let mut pending = responses.into_iter();
            'outer: while !stop_for_thread.load(Ordering::Relaxed) {
                let Some(response) = pending.next() else {
                    break;
                };
                let stream = loop {
                    if stop_for_thread.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(_) => return,
                    }
                };
                serve(stream, response, &requests_for_thread);
            }
        });
        Self {
            addr,
            stop,
            requests,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(mut stream: TcpStream, response: Response, requests: &Arc<Mutex<Vec<RecordedRequest>>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    let _ = reader.read_line(&mut request_line);
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => headers.push(line.trim_end().to_owned()),
            Err(_) => break,
        }
    }
    requests.lock().unwrap().push(RecordedRequest {
        line: request_line.trim_end().to_owned(),
        headers,
    });
    match response {
        Response::Ok(body) => send_response(&mut stream, 200, "OK", &[], &body),
        Response::Status(status, body) => {
            send_response(&mut stream, status, "Error", &[], &body);
        }
        Response::Redirect(location) => {
            send_response(&mut stream, 302, "Found", &[("Location", &location)], &[])
        }
        Response::Raw(bytes) => {
            let _ = stream.write_all(&bytes);
        }
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn send_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    // The client can legitimately abort after seeing the headers (for example
    // on an error status or a size-limit rejection), so a failed write is not
    // a test failure.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

#[test]
fn fetch_requires_network_and_filesystem_permissions_before_io() {
    let denied = runtime(false, true);
    assert!(
        denied
            .run_module("denied-fetch.js", "import 'neo:fetch';")
            .is_err()
    );

    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.bin");
    let server = TestServer::start(vec![Response::Ok(b"body".to_vec())]);
    let js = runtime(true, false);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         let reads = 0, calls = 0;
         const validationError = Error('option getter failure');
         try {{
             download({url}, {path}, {{ get progress() {{ throw validationError; }} }});
             throw Error('expected validation before permission check');
         }} catch (error) {{
             if (error !== validationError) throw error;
         }}
         try {{
             download({url}, {path}, {{
                 get progress() {{
                     ++reads;
                     return async () => {{ ++calls; }};
                 }},
             }});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}
         if (reads !== 1 || calls !== 0) throw Error('unexpected option or callback invocation');",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_PERMISSION");
    assert!(!destination.exists());
    assert_eq!(server.request_count(), 0);
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_defaults_do_not_use_inherited_options() {
    let server = TestServer::start(vec![
        Response::Ok(b"body".to_vec()),
        Response::Ok(b"body".to_vec()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let implicit = dir.path().join("implicit.bin");
    let explicit = dir.path().join("explicit.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         let calls = 0, inheritedReads = 0, ownReads = 0;
         Object.defineProperty(Object.prototype, 'progress', {{ value: async () => {{ ++calls; }} }});
         const a = download({url}, {implicit});
         Object.defineProperty(Object.prototype, 'timeoutMs', {{ get() {{ ++inheritedReads; return 0; }} }});
         const options = Object.create({{ maxBytes: 0 }});
         Object.defineProperty(options, 'overwrite', {{ get() {{ ++ownReads; return undefined; }} }});
         const b = download({url}, {explicit}, options);
         if (a.bytes !== 4 || b.bytes !== 4 || calls !== 0 || inheritedReads !== 0 || ownReads !== 1) {{
             throw Error('unexpected option resolution');
         }}",
        url = js_string(&server.url("/file")),
        implicit = path_string(&implicit),
        explicit = path_string(&explicit),
    );
    js.run_module("fetch-own-options.js", &script).unwrap();
    assert_eq!(server.request_count(), 2);
    assert_eq!(std::fs::read(implicit).unwrap(), b"body");
    assert_eq!(std::fs::read(explicit).unwrap(), b"body");
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_downloads_with_observable_progress_and_result() {
    let body: Vec<u8> = (0..200_000u32).map(|value| (value % 251) as u8).collect();
    let server = TestServer::start(vec![Response::Ok(body.clone())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("download.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.events = [];
         const result = download({url}, {path}, {{
             progress(received, total) {{
                 globalThis.events.push([received, total]);
             }},
         }});
         globalThis.result = result;",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    js.run_module("fetch-success.js", &script).unwrap();

    assert_eq!(std::fs::read(&destination).unwrap(), body);
    assert_eq!(server.request_count(), 1);
    assert_no_temporary_files(dir.path());
    assert_eq!(
        js.eval("String(globalThis.result.bytes)").unwrap(),
        "200000"
    );
    assert_eq!(js.eval("String(globalThis.result.status)").unwrap(), "200");

    let events = js.eval("String(globalThis.events.length)").unwrap();
    assert!(events.parse::<usize>().unwrap() >= 2);
    assert_eq!(js.eval("String(globalThis.events[0][0])").unwrap(), "0");
    assert_eq!(
        js.eval("String(globalThis.events[0][1])").unwrap(),
        "200000"
    );
    assert_eq!(
        js.eval("String(globalThis.events[globalThis.events.length - 1][0])")
            .unwrap(),
        "200000"
    );
    assert_eq!(
        js.eval(
            "String(globalThis.events.every((event, index) => \
             index === 0 || event[0] >= globalThis.events[index - 1][0]))"
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn fetch_reports_bad_statuses_without_creating_or_replacing_the_destination() {
    let server = TestServer::start(vec![
        Response::Status(404, b"missing".to_vec()),
        Response::Status(503, b"unavailable".to_vec()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("status.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.results = [];
         for (const url of [{first}, {second}]) {{
             try {{
                 download(url, {path});
             }} catch (error) {{
                 globalThis.results.push([error.code, error.status, error.message]);
             }}
         }}",
        first = js_string(&server.url("/missing")),
        second = js_string(&server.url("/unavailable")),
        path = path_string(&destination),
    );
    js.run_module("fetch-status.js", &script).unwrap();
    assert_eq!(js.eval("String(globalThis.results.length)").unwrap(), "2");
    assert_eq!(js.eval("globalThis.results[0][0]").unwrap(), "FETCH_STATUS");
    assert_eq!(js.eval("String(globalThis.results[0][1])").unwrap(), "404");
    assert_eq!(js.eval("globalThis.results[1][0]").unwrap(), "FETCH_STATUS");
    assert_eq!(js.eval("String(globalThis.results[1][1])").unwrap(), "503");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_enforces_the_size_limit_for_declared_and_streamed_bodies() {
    let streamed = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n0123456789abcdef".to_vec();
    let declared = b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n".to_vec();
    let server = TestServer::start(vec![Response::Raw(streamed), Response::Raw(declared)]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("limit.bin");
    let js = runtime(true, true);
    for (index, url) in [server.url("/streamed"), server.url("/declared")]
        .into_iter()
        .enumerate()
    {
        let script = format!(
            "import {{ download }} from 'neo:fetch';
             try {{
                 download({url}, {path}, {{ maxBytes: 8 }});
                 globalThis.lastCode = 'none';
             }} catch (error) {{
                 globalThis.lastCode = error.code;
             }}",
            url = js_string(&url),
            path = path_string(&destination),
        );
        js.run_module(&format!("fetch-limit-{index}.js"), &script)
            .unwrap();
        assert_eq!(js.eval("globalThis.lastCode").unwrap(), "FETCH_LIMIT");
        assert!(!destination.exists());
        assert_no_temporary_files(dir.path());
    }
}

#[test]
fn fetch_reports_truncated_responses_and_cleans_up() {
    let response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec();
    let server = TestServer::start(vec![Response::Raw(response)]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("truncated.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/truncated")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_TRUNCATED");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_follows_redirects_and_enforces_the_redirect_limit() {
    let body = b"redirected body".to_vec();
    let server = TestServer::start(vec![
        Response::Redirect("/final".to_owned()),
        Response::Ok(body.clone()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("redirect.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         const result = download({url}, {path});
         globalThis.result = result;",
        url = js_string(&server.url("/start")),
        path = path_string(&destination),
    );
    js.run_module("fetch-redirect.js", &script).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), body);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].line.contains("GET /start"), "{requests:?}");
    assert!(requests[1].line.contains("GET /final"), "{requests:?}");
    assert_no_authorization(&requests);
    assert_eq!(js.eval("String(globalThis.result.status)").unwrap(), "200");
    assert_no_temporary_files(dir.path());

    let limiter = TestServer::start(vec![
        Response::Redirect("/again".to_owned()),
        Response::Redirect("/again".to_owned()),
        Response::Redirect("/again".to_owned()),
        Response::Redirect("/again".to_owned()),
        Response::Redirect("/again".to_owned()),
        Response::Redirect("/again".to_owned()),
    ]);
    let limited_destination = dir.path().join("limited.bin");
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&limiter.url("/loop")),
        path = path_string(&limited_destination),
    );
    assert_error_code(&js, &script, "FETCH_REDIRECT");
    assert!(!limited_destination.exists());
    assert_no_temporary_files(dir.path());

    // A redirect is still subject to the http/https rule.
    let file_redirect =
        TestServer::start(vec![Response::Redirect("file:///etc/passwd".to_owned())]);
    let redirected_destination = dir.path().join("redirect-scheme.bin");
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&file_redirect.url("/start")),
        path = path_string(&redirected_destination),
    );
    assert_error_code(&js, &script, "FETCH_INVALID_URL");
    assert!(!redirected_destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_rejects_unsupported_urls_before_creating_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("never.bin");
    let js = runtime(true, true);
    let urls = [
        "file:///etc/passwd",
        "ftp://example.test/file",
        "javascript:alert(1)",
        "http://",
        "https://",
        "not a url",
    ];
    let list = urls
        .iter()
        .map(|url| js_string(url))
        .collect::<Vec<_>>()
        .join(", ");
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.codes = [];
         for (const url of [{list}]) {{
             try {{
                 download(url, {path});
                 globalThis.codes.push('none');
             }} catch (error) {{
                 globalThis.codes.push(error.code);
             }}
         }}",
        path = path_string(&destination),
    );
    js.run_module("fetch-urls.js", &script).unwrap();
    for index in 0..urls.len() {
        assert_eq!(
            js.eval(&format!("globalThis.codes[{index}]")).unwrap(),
            "FETCH_INVALID_URL",
            "URL index {index}"
        );
    }
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_protects_an_existing_destination_and_overwrite_is_explicit() {
    let server = TestServer::start(vec![Response::Ok(b"new contents".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing.bin");
    std::fs::write(&destination, b"old contents").unwrap();
    let js = runtime(true, true);

    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_EXISTS");
    assert_eq!(std::fs::read(&destination).unwrap(), b"old contents");
    assert_eq!(server.request_count(), 0);

    let script = format!(
        "import {{ download }} from 'neo:fetch';
         const result = download({url}, {path}, {{ overwrite: true }});
         globalThis.result = result;",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    js.run_module("fetch-overwrite.js", &script).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new contents");
    assert_eq!(server.request_count(), 1);
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_rejects_a_directory_destination() {
    let server = TestServer::start(vec![Response::Ok(b"body".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("directory");
    std::fs::create_dir(&destination).unwrap();
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_PATH");
    assert!(destination.is_dir());
    assert_eq!(server.request_count(), 0);
}

#[test]
fn fetch_callback_failure_aborts_the_download_and_cleans_up() {
    let body = vec![7u8; 128 * 1024];
    let server = TestServer::start(vec![Response::Ok(body)]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("callback.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path}, {{
                 progress(received) {{
                     if (received > 0) throw new Error('injected callback failure');
                 }},
             }});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             if (!(error instanceof Error) || error instanceof TypeError ||
                 error.message !== 'the progress callback threw' ||
                 Object.hasOwn(error, 'status')) throw Error('unexpected callback error');
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_CALLBACK");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_rejects_direct_async_progress_before_invocation_or_io() {
    let server = TestServer::start(vec![Response::Ok(b"unused".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("async.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         let calls = 0;
         const callback = async () => {{ ++calls; throw Error('must not run'); }};
         Object.setPrototypeOf(callback, null);
         try {{
             download({url}, {path}, {{ progress: callback }});
         }} catch (error) {{
             if (!(error instanceof Error) || error instanceof TypeError ||
                 error.message !== 'the progress callback must be synchronous' ||
                 Object.hasOwn(error, 'status')) throw Error('unexpected async error');
             globalThis.lastCode = error.code;
         }}
         if (calls !== 0) throw Error('async callback ran');",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    assert_error_code(&js, &script, "FETCH_CALLBACK_ASYNC");
    assert_eq!(server.request_count(), 0);
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_callback_error_is_preserved_if_temporary_cleanup_cannot_finish() {
    let server = TestServer::start(vec![Response::Ok(b"unused".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.bin");
    std::fs::write(&destination, b"original").unwrap();
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         import {{ readDirSync, writeText }} from 'neo:fs';
         import {{ posix, win32 }} from 'neo:path';
         const pathApi = {path_api};
         try {{
             download({url}, {path}, {{
                 overwrite: true,
                 progress() {{
                     const temp = readDirSync({dir}).find(entry => entry.name.startsWith('.neo-download-'));
                     writeText(pathApi.join({dir}, temp.name, 'keep'), 'not owned by the download');
                     return Promise.resolve();
                 }},
             }});
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
        dir = path_string(dir.path()),
        path_api = if cfg!(windows) { "win32" } else { "posix" },
    );
    assert_error_code(&js, &script, "FETCH_CALLBACK_ASYNC");
    assert_eq!(server.request_count(), 1);
    assert_eq!(std::fs::read(&destination).unwrap(), b"original");
    let residue = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".neo-download-")
        })
        .expect("nonempty temporary directory should remain");
    assert_eq!(
        std::fs::read(residue.path().join("keep")).unwrap(),
        b"not owned by the download"
    );
}

#[test]
fn fetch_checks_progress_returns_without_running_getters_or_traps() {
    let server = TestServer::start(
        (0..10)
            .map(|_| Response::Ok(b"downloaded".to_vec()))
            .collect(),
    );
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("progress.bin");
    let fresh = dir.path().join("fresh.bin");
    std::fs::write(&destination, b"original").unwrap();
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         import {{ readText, readDirSync }} from 'neo:fs';
         const url = {url}, path = {path}, dir = {dir};
         let inspections = 0;
         const trap = () => {{ ++inspections; throw Error('must not run'); }};
         const proxy = new Proxy({{then() {{}}}}, {{
             get: trap, getOwnPropertyDescriptor: trap, getPrototypeOf: trap,
         }});
         const promise = Promise.reject(Error('private reason'));
         Object.defineProperties(promise, {{
             then: {{get: trap}}, constructor: {{get: trap}},
         }});
         function thenableFunction() {{}}
         thenableFunction.then = trap;
         const cases = [
             () => promise,
             () => Promise.resolve(),
             (async () => {{ throw Error('bound rejection'); }}).bind(null),
             () => Object.create({{then: trap}}),
             () => Object.create({{get then() {{ return trap(); }}}}),
             () => proxy,
             () => Object.create(proxy),
             () => thenableFunction,
         ];
         for (const callback of cases) {{
             let calls = 0;
             let failure;
             try {{
                 download(url, path, {{
                     overwrite: true,
                     progress(received) {{
                         ++calls;
                         if (received > 0) return callback();
                     }},
                 }});
             }} catch (error) {{ failure = error; }}
             if (failure?.code !== 'FETCH_CALLBACK_ASYNC' ||
                 failure.message !== 'the progress callback must be synchronous' ||
                 Object.hasOwn(failure, 'status') || calls !== 2) throw Error('wrong failure');
             if (readText(path) !== 'original' ||
                 readDirSync(dir).some(entry => entry.name.startsWith('.neo-download-'))) {{
                 throw Error('unfinished cancellation');
             }}
         }}
         let ranJob = false;
         let pendingFailure;
         try {{
             download(url, {fresh}, {{
                 progress: () => Promise.resolve().then(() => {{
                     ranJob = true;
                     throw Error('later rejection');
                 }}),
             }});
         }} catch (error) {{ pendingFailure = error; }}
         if (pendingFailure?.code !== 'FETCH_CALLBACK_ASYNC' || ranJob) {{
             throw Error('did not reject synchronously');
         }}
         if (readDirSync(dir).some(entry => entry.name !== 'progress.bin')) {{
             throw Error('published new destination or left temporary output');
         }}
         await Promise.resolve();
         if (!ranJob) throw Error('unexpected cancellation of JS work');
         const result = download(url, path, {{
             overwrite: true,
             progress: () => ({{then: 1, get unrelated() {{ return trap(); }}}}),
         }});
         if (inspections !== 0 || result.bytes !== 10) throw Error('unexpected inspection or result');",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
        dir = path_string(dir.path()),
        fresh = path_string(&fresh),
    );
    js.run_module("fetch-callback-results.js", &script).unwrap();
    assert_eq!(server.request_count(), 10);
    assert_eq!(std::fs::read(destination).unwrap(), b"downloaded");
    assert!(!fresh.exists());
    assert_no_temporary_files(dir.path());
}

#[cfg(unix)]
#[test]
fn fetch_progress_window_substitution_is_rejected_without_clobber() {
    use std::time::Instant;

    let server = TestServer::start(vec![Response::Ok(b"server body".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("substituted.bin");
    std::fs::write(&destination, b"original").unwrap();
    let js = runtime(true, true);

    // The progress callback is a same-user window. A watcher scans the
    // destination directory during that window and replaces the temporary
    // file, exactly as a callback or another same-user process could.
    let stop = Arc::new(AtomicBool::new(false));
    let attacker = Arc::new(Mutex::new(None::<std::path::PathBuf>));
    let watcher_stop = Arc::clone(&stop);
    let watcher_attacker = Arc::clone(&attacker);
    let parent = dir.path().to_path_buf();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !watcher_stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            if let Ok(entries) = std::fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    if !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".neo-download-")
                    {
                        continue;
                    }
                    let path = entry.path().join("download");
                    if std::fs::remove_file(&path).is_ok()
                        && std::fs::write(&path, b"attacker").is_ok()
                    {
                        *watcher_attacker.lock().unwrap() = Some(path);
                        return;
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    });

    let script = format!(
        "import {{ download }} from 'neo:fetch';
         try {{
             download({url}, {path}, {{
                 timeoutMs: 5000,
                 overwrite: true,
                 progress(received) {{
                     if (received === 0) {{
                         const until = Date.now() + 1000;
                         while (Date.now() < until) {{}}
                     }}
                 }},
             }});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    js.run_module("fetch-substitution.js", &script).unwrap();
    stop.store(true, Ordering::Relaxed);
    watcher.join().unwrap();

    assert_eq!(js.eval("globalThis.lastCode").unwrap(), "FETCH_TEMP");
    assert_eq!(std::fs::read(&destination).unwrap(), b"original");
    let attacker = attacker
        .lock()
        .unwrap()
        .clone()
        .expect("watcher did not substitute the temporary file");
    assert_eq!(std::fs::read(&attacker).unwrap(), b"attacker");
}

#[test]
fn fetch_deadline_is_checked_after_a_slow_initial_callback_for_an_empty_body() {
    let server = TestServer::start(vec![Response::Ok(Vec::new())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("empty-timeout.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.ran = false;
         try {{
             download({url}, {path}, {{
                 timeoutMs: 1000,
                 progress() {{
                     globalThis.ran = true;
                     const until = Date.now() + 1500;
                     while (Date.now() < until) {{}}
                 }},
             }});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/empty")),
        path = path_string(&destination),
    );
    js.run_module("fetch-empty-timeout.js", &script).unwrap();
    assert_eq!(js.eval("globalThis.lastCode").unwrap(), "FETCH_TIMEOUT");
    assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_deadline_is_checked_after_a_slow_final_callback() {
    let server = TestServer::start(vec![Response::Ok(b"small body".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("final-timeout.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.ran = false;
         try {{
             download({url}, {path}, {{
                 timeoutMs: 1000,
                 progress(received) {{
                     if (received > 0) {{
                         globalThis.ran = true;
                         const until = Date.now() + 1500;
                         while (Date.now() < until) {{}}
                     }}
                 }},
             }});
             globalThis.lastCode = 'none';
         }} catch (error) {{
             globalThis.lastCode = error.code;
         }}",
        url = js_string(&server.url("/small")),
        path = path_string(&destination),
    );
    js.run_module("fetch-final-timeout.js", &script).unwrap();
    assert_eq!(js.eval("globalThis.lastCode").unwrap(), "FETCH_TIMEOUT");
    assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[cfg(unix)]
#[test]
fn fetch_publishes_a_private_destination_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let server = TestServer::start(vec![Response::Ok(b"private".to_vec())]);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("private.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         download({url}, {path});",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    js.run_module("fetch-private.js", &script).unwrap();
    let mode = std::fs::metadata(&destination)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode & 0o077, 0);
    assert_eq!(mode & 0o600, 0o600);
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_progress_callback_can_start_another_download() {
    let outer = vec![3u8; 96 * 1024];
    let inner = b"inner body".to_vec();
    let server = TestServer::start(vec![
        Response::Ok(outer.clone()),
        Response::Ok(inner.clone()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let outer_path = dir.path().join("outer.bin");
    let inner_path = dir.path().join("inner.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.ranInner = false;
         const outerResult = download({outer_url}, {outer_path}, {{
             progress(received) {{
                 if (!globalThis.ranInner && received > 0) {{
                     globalThis.ranInner = true;
                     globalThis.innerResult = download({inner_url}, {inner_path});
                 }}
             }},
         }});
         globalThis.outerResult = outerResult;",
        outer_url = js_string(&server.url("/outer")),
        outer_path = path_string(&outer_path),
        inner_url = js_string(&server.url("/inner")),
        inner_path = path_string(&inner_path),
    );
    js.run_module("fetch-reentrant.js", &script).unwrap();
    assert_eq!(std::fs::read(&outer_path).unwrap(), outer);
    assert_eq!(std::fs::read(&inner_path).unwrap(), inner);
    assert_eq!(js.eval("String(globalThis.ranInner)").unwrap(), "true");
    assert_eq!(server.request_count(), 2);
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_sends_no_authorization_and_rejects_initial_userinfo_before_io() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("credential.bin");
    let js = runtime(true, true);

    // A normal request carries no Authorization header.
    let server = TestServer::start(vec![Response::Ok(b"body".to_vec())]);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         download({url}, {path});",
        url = js_string(&server.url("/file")),
        path = path_string(&destination),
    );
    js.run_module("fetch-no-auth.js", &script).unwrap();
    assert_eq!(server.request_count(), 1);
    assert_no_authorization(&server.requests());
    std::fs::remove_file(&destination).unwrap();

    // Initial userinfo, including empty and empty-password forms, is rejected
    // before any request is sent.
    let server = TestServer::start(vec![]);
    let authority = server.addr.to_string();
    let urls = [
        format!("http://user:TOPSECRET@{authority}/secret"),
        format!("http://user@{authority}/secret"),
        format!("http://@{authority}/secret"),
        format!(" http://@{authority}/secret"),
        format!("ht\ttp://@{authority}/secret"),
        format!("http://:TOPSECRET@{authority}/secret"),
        format!("http://user:@{authority}/secret"),
        format!("http:/@{authority}/secret"),
        format!("http:@{authority}/secret"),
        format!("http://user%3Apass@{authority}/secret"),
    ];
    let list = urls
        .iter()
        .map(|url| js_string(url))
        .collect::<Vec<_>>()
        .join(", ");
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.results = [];
         for (const url of [{list}]) {{
             try {{
                 download(url, {path});
                 globalThis.results.push(['none', 'none']);
             }} catch (error) {{
                 globalThis.results.push([error.code, error.message]);
             }}
         }}",
        path = path_string(&destination),
    );
    js.run_module("fetch-userinfo-initial.js", &script).unwrap();
    for index in 0..urls.len() {
        assert_eq!(
            js.eval(&format!("globalThis.results[{index}][0]")).unwrap(),
            "FETCH_INVALID_URL",
            "URL index {index}"
        );
        let message = js.eval(&format!("globalThis.results[{index}][1]")).unwrap();
        assert!(!message.contains("TOPSECRET"), "{message}");
        assert!(!message.contains("user"), "{message}");
        assert!(!message.contains("127.0.0.1"), "{message}");
    }
    assert_eq!(server.request_count(), 0);
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

#[test]
fn fetch_rejects_a_redirect_to_userinfo_before_the_next_request() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("redirect-userinfo.bin");
    let js = runtime(true, true);
    let templates = [
        "http://user:TOPSECRET@{}/secret",
        "http://@{}/secret",
        "//user:TOPSECRET@{}/secret",
        "//@{}/secret",
        "http:/@{}/secret",
        "  http://@{}/secret  ",
        "ht\ttp://@{}/secret",
        "/\\@{}/secret",
        "/\\/@{}/secret",
    ];
    for (index, template) in templates.into_iter().enumerate() {
        let server = TestServer::start_with(|addr| {
            vec![Response::Redirect(
                template.replace("{}", &addr.to_string()),
            )]
        });
        let script = format!(
            "import {{ download }} from 'neo:fetch';
             try {{
                 download({url}, {path});
                 globalThis.lastCode = 'none';
                 globalThis.message = 'none';
             }} catch (error) {{
                 globalThis.lastCode = error.code;
                 globalThis.message = error.message;
             }}",
            url = js_string(&server.url("/start")),
            path = path_string(&destination),
        );
        js.run_module(&format!("fetch-redirect-userinfo-{index}.js"), &script)
            .unwrap();
        assert_eq!(js.eval("globalThis.lastCode").unwrap(), "FETCH_INVALID_URL");
        assert_eq!(server.request_count(), 1);
        assert_no_authorization(&server.requests());
        let message = js.eval("globalThis.message").unwrap();
        assert!(!message.contains("TOPSECRET"), "{message}");
        assert!(!message.contains("user"), "{message}");
        assert!(!destination.exists());
        assert_no_temporary_files(dir.path());
    }
}

#[test]
fn mixed_slash_redirects_reject_empty_userinfo_without_contacting_target() {
    let templates = [
        "/\\/@{}/file",
        "\\//@{}/file",
        "/\\\\/@{}/file",
        "\\/\\/@{}/file",
        " \t/\\/@{}/file",
        "/\t\\/@{}/file",
        "\u{1}/\\/@{}/file",
    ];
    let dir = tempfile::tempdir().unwrap();
    for (index, template) in templates.into_iter().enumerate() {
        let target = TestServer::start(vec![Response::Ok(b"should not be requested".to_vec())]);
        let origin = TestServer::start(vec![Response::Redirect(
            template.replace("{}", &target.addr.to_string()),
        )]);
        let destination = dir.path().join(format!("invalid-{index}"));
        let js = runtime(true, true);
        let script = format!(
            "import {{ download }} from 'neo:fetch'; try {{ download({}, {}); globalThis.lastCode = 'success'; }} catch (e) {{ globalThis.lastCode = e.code; }}",
            js_string(&origin.url("/start")),
            path_string(&destination),
        );
        js.run_module(&format!("mixed-invalid-{index}.js"), &script)
            .unwrap();
        let code = js.eval("lastCode").unwrap();
        if template.starts_with('\u{1}') {
            // Invalid HTTP header controls may be rejected by the transport
            // before URL parsing rather than by the URL policy.
            assert!(
                ["FETCH_INVALID_URL", "FETCH_NETWORK", "FETCH_STATUS"].contains(&code.as_str()),
                "{code}"
            );
        } else {
            assert_eq!(code, "FETCH_INVALID_URL", "{template:?}");
        }
        assert_eq!(origin.request_count(), 1);
        assert_eq!(target.request_count(), 0, "{template:?}");
        assert!(!destination.exists());
        assert_no_temporary_files(dir.path());
    }
}

#[test]
fn normal_and_mixed_slash_redirects_without_userinfo_still_work() {
    for (index, template) in [
        "//{}/file",
        "/\\/{}/file",
        "\\/\\/{}/file",
        " \t/\\/{}/file",
    ]
    .into_iter()
    .enumerate()
    {
        let target = TestServer::start(vec![Response::Ok(b"redirected".to_vec())]);
        let origin = TestServer::start(vec![Response::Redirect(
            template.replace("{}", &target.addr.to_string()),
        )]);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("data");
        let js = runtime(true, true);
        let script = format!(
            "import {{ download }} from 'neo:fetch'; download({}, {});",
            js_string(&origin.url("/start")),
            path_string(&destination),
        );
        js.run_module(&format!("mixed-valid-{index}.js"), &script)
            .unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"redirected");
        assert_eq!(target.request_count(), 1);
    }
}

#[test]
fn fetch_rejects_invalid_arguments_and_options_with_type_errors() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("types.bin");
    let js = runtime(true, true);
    let script = format!(
        "import {{ download }} from 'neo:fetch';
         globalThis.typeErrors = 0;
         const cases = [
             () => download(123, {path}),
             () => download('http://example.test/', {{}}),
             () => download('http://example.test/', {path}, null),
             () => download('http://example.test/', {path}, {{ maxBytes: -1 }}),
             () => download('http://example.test/', {path}, {{ maxBytes: 1.5 }}),
             () => download('http://example.test/', {path}, {{ timeoutMs: 0 }}),
             () => download('http://example.test/', {path}, {{ overwrite: 'yes' }}),
             () => download('http://example.test/', {path}, {{ progress: 1 }}),
         ];
         for (const run of cases) {{
             try {{
                 run();
             }} catch (error) {{
                 if (error instanceof TypeError) globalThis.typeErrors += 1;
             }}
         }}",
        path = path_string(&destination),
    );
    js.run_module("fetch-types.js", &script).unwrap();
    assert_eq!(js.eval("String(globalThis.typeErrors)").unwrap(), "8");
    assert!(!destination.exists());
    assert_no_temporary_files(dir.path());
}

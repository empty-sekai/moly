//! The server addresses every game target admits.

/// The server a native start uses when `MOLY_SERVER_URL` is unset.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:15800";

/// Whether `value` is a server address: `http://` or `https://`, then a host,
/// an optional port, and no path, no query, no credentials and no
/// whitespace.
pub fn is_server_address(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
    else {
        return false;
    };
    if value.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
        return false;
    }
    if rest.contains(['/', '?', '#', '@']) {
        return false;
    }
    // A bracketed IPv6 address, else a registered name or an IPv4 address;
    // then an optional `:port`.
    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let Some((address, after)) = inner.split_once(']') else {
            return false;
        };
        if address.is_empty() || !address.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.') {
            return false;
        }
        match after {
            "" => (address, None),
            _ => match after.strip_prefix(':') {
                Some(port) => (address, Some(port)),
                None => return false,
            },
        }
    } else {
        let (host, port) = match rest.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (rest, None),
        };
        let name = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_';
        if host.is_empty() || !host.bytes().all(name) {
            return false;
        }
        (host, port)
    };
    debug_assert!(!host.is_empty());
    match port {
        Some(port) => {
            !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u16>().is_ok()
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_server_address_is_admitted() {
        assert!(is_server_address(DEFAULT_SERVER_URL));
    }

    #[test]
    fn only_a_scheme_host_and_port_are_admitted() {
        let admitted = [
            "http://127.0.0.1:15800",
            "http://localhost:15800",
            "https://example.invalid",
            "https://example.invalid:443",
            "http://[::1]:15800",
            "http://[::1]",
        ];
        let refused = [
            "",
            "127.0.0.1:15800",
            "socket://127.0.0.1:15800",
            "http://",
            "http://:15800",
            "http://127.0.0.1:",
            "http://127.0.0.1:port",
            "http://127.0.0.1:-1",
            "http://127.0.0.1:70000",
            "http://user:pass@127.0.0.1:15800",
            "http://127.0.0.1:15800/rpc",
            "http://127.0.0.1:15800?x=1",
            "http://127.0.0.1:15800#top",
            "http://127.0.0.1 :15800",
            "http://127.0.0.1:15800 ",
            "http://::1",
            "ftp://127.0.0.1:15800",
        ];
        for value in admitted {
            assert!(is_server_address(value), "{value}");
        }
        for value in refused {
            assert!(!is_server_address(value), "{value}");
        }
    }
}

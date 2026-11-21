//! `Util.parseAndValidateUntrustedUri`: `new URI(uri)` (java.net.URI's
//! RFC 2396 parser), then a scheme of `http` or `https`. The messages are
//! `URISyntaxException.getMessage`: `reason at index N: input`, or
//! `reason: input` for the protocol checks.

/// java.net.URI's character classes.
const MARK: &str = "-_.!~*'()";
const RESERVED: &str = ";/?:@&=+$,[]";
#[derive(Clone, Copy)]
struct Class {
    chars: &'static str,
    alphanum: bool,
    /// `L_ESCAPED`: `%XX` and the non-ASCII "other" characters.
    escaped: bool,
    /// The `unreserved` set (alphanumerics and the marks).
    unreserved: bool,
}
const URIC: Class = Class {
    chars: RESERVED,
    alphanum: true,
    escaped: true,
    unreserved: true,
};
const PATH: Class = Class {
    chars: ":@&=+$,;/",
    ..URIC
};
const SCHEME: Class = Class {
    chars: "+-.",
    alphanum: true,
    escaped: false,
    unreserved: false,
};
const REG_NAME: Class = Class {
    chars: "$,;:@&=+",
    ..URIC
};
const SERVER: Class = Class {
    chars: ";:&=+$,.@[]",
    ..URIC
};
const SERVER_PERCENT: Class = Class {
    chars: ";:&=+$,.@[]%",
    ..URIC
};

struct Parser<'a> {
    input: &'a str,
    chars: Vec<char>,
}

impl Parser<'_> {
    /// `URISyntaxException(input, reason, index)`. Java's index counts UTF-16
    /// units; the inputs that reach it are almost always ASCII.
    fn fail(&self, reason: &str, p: usize) -> String {
        let index: usize = self.chars[..p].iter().map(|c| c.len_utf16()).sum();
        format!("{reason} at index {index}: {}", self.input)
    }
    fn at(&self, p: usize, c: char) -> bool {
        self.chars.get(p) == Some(&c)
    }
    /// `scan(start, end, chars)`: the first of `chars`, else `end`.
    fn find(&self, start: usize, end: usize, stop: &str) -> usize {
        (start..end)
            .find(|&p| stop.contains(self.chars[p]))
            .unwrap_or(end)
    }
    fn matches(c: char, class: Class) -> bool {
        c.is_ascii()
            && ((class.alphanum && c.is_ascii_alphanumeric())
                || (class.unreserved && MARK.contains(c))
                || class.chars.contains(c))
    }
    /// `scan(start, n, lowMask, highMask)` with `scanEscape`.
    fn scan(&self, start: usize, end: usize, class: Class) -> Result<usize, String> {
        let mut p = start;
        while p < end {
            let c = self.chars[p];
            if Self::matches(c, class) {
                p += 1;
                continue;
            }
            if class.escaped {
                if c == '%' {
                    let hex = |q: usize| q < end && self.chars[q].is_ascii_hexdigit();
                    if hex(p + 1) && hex(p + 2) {
                        p += 3;
                        continue;
                    }
                    return Err(self.fail("Malformed escape pair", p));
                }
                // Character.isSpaceChar / isISOControl.
                if !c.is_ascii() && !c.is_whitespace() && !c.is_control() {
                    p += 1;
                    continue;
                }
            }
            break;
        }
        Ok(p)
    }
    fn check(&self, start: usize, end: usize, class: Class, what: &str) -> Result<(), String> {
        let p = self.scan(start, end, class)?;
        if p < end {
            return Err(self.fail(&format!("Illegal character in {what}"), p));
        }
        Ok(())
    }
    fn parse(&self) -> Result<Option<String>, String> {
        let n = self.chars.len();
        // scan(0, n, "/?#", ":"): a scheme ends at the first `:` before any
        // of `/?#`.
        let colon = (0..n)
            .take_while(|&p| !"/?#".contains(self.chars[p]))
            .find(|&p| self.chars[p] == ':');
        let mut scheme = None;
        let mut p = match colon {
            Some(0) => return Err(self.fail("Expected scheme name", 0)),
            Some(c) => {
                if !self.chars[0].is_ascii_alphabetic() {
                    return Err(self.fail("Illegal character in scheme name", 0));
                }
                self.check(1, c, SCHEME, "scheme name")?;
                scheme = Some(self.chars[..c].iter().collect());
                let p = c + 1;
                if self.at(p, '/') {
                    self.hierarchical(p)?
                } else {
                    let q = self.find(p, n, "#");
                    if q <= p {
                        return Err(self.fail("Expected scheme-specific part", p));
                    }
                    self.check(p, q, URIC, "opaque part")?;
                    q
                }
            }
            None => self.hierarchical(0)?,
        };
        if self.at(p, '#') {
            self.check(p + 1, n, URIC, "fragment")?;
            p = n;
        }
        if p < n {
            return Err(self.fail("Expected end of URI", p));
        }
        Ok(scheme)
    }
    fn hierarchical(&self, start: usize) -> Result<usize, String> {
        let n = self.chars.len();
        let mut p = start;
        if self.at(p, '/') && self.at(p + 1, '/') {
            p += 2;
            let q = self.find(p, n, "/?#");
            if q > p {
                p = self.authority(p, q)?;
            } else if q >= n {
                return Err(self.fail("Expected authority", p));
            }
        }
        let q = self.find(p, n, "?#");
        self.check(p, q, PATH, "path")?;
        p = q;
        if self.at(p, '?') {
            let q = self.find(p + 1, n, "#");
            self.check(p + 1, q, URIC, "query")?;
            p = q;
        }
        Ok(p)
    }
    /// `parseAuthority`: a registry-based authority is any `reg_name`; one
    /// with `[`/`]` must parse as a server (`[IPv6]`).
    fn authority(&self, p: usize, n: usize) -> Result<usize, String> {
        let bracket = self.find(p, n, "]") < n;
        let server_class = if bracket { SERVER_PERCENT } else { SERVER };
        let server_chars = self.scan(p, n, server_class)? == n;
        let qreg = self.scan(p, n, REG_NAME)?;
        if qreg == n {
            return Ok(n);
        }
        if !server_chars {
            return Err(self.fail("Illegal character in authority", qreg));
        }
        self.server(p, n)?;
        Ok(n)
    }
    /// `parseServer` for an authority holding `[`: `[userinfo@][IPv6][:port]`.
    fn server(&self, start: usize, n: usize) -> Result<(), String> {
        let mut p = start;
        let at = (p..n).find(|&q| self.chars[q] == '@');
        if let Some(at) = at {
            self.check(p, at, SERVER, "user info")?;
            p = at + 1;
        }
        if !self.at(p, '[') {
            return Err(self.fail("Expected end of authority", self.find(p, n, "[]")));
        }
        p += 1;
        let Some(close) = (p..n).find(|&q| self.chars[q] == ']') else {
            return Err(self.fail("Expected closing bracket for IPv6 address", n));
        };
        let address: String = self.chars[p..close].iter().collect();
        // An IPv6 literal may carry a `%scope` after the address.
        let address = address.split_once('%').map_or(&*address, |(a, _)| a);
        if address.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(self.fail("Malformed IPv6 address", p));
        }
        p = close + 1;
        if self.at(p, ':') {
            p += 1;
            while p < n && self.chars[p].is_ascii_digit() {
                p += 1;
            }
        }
        if p < n {
            return Err(self.fail("Expected end of authority", p));
        }
        Ok(())
    }
}

/// `Util.parseAndValidateUntrustedUri` as `ExtraCodecs.UNTRUSTED_URI` reports
/// it: `Err` holds the exception's message.
pub(crate) fn parse_untrusted_uri(uri: &str) -> Result<(), String> {
    let parser = Parser {
        input: uri,
        chars: uri.chars().collect(),
    };
    match parser.parse()? {
        None => Err(format!("Missing protocol in URI: {uri}: {uri}")),
        // ALLOWED_UNTRUSTED_LINK_PROTOCOLS, scheme.toLowerCase(Locale.ROOT).
        Some(s) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https") => Ok(()),
        Some(_) => Err(format!("Unsupported protocol in URI: {uri}: {uri}")),
    }
}

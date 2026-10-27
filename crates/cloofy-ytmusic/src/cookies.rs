//! Cookies in the Netscape `cookies.txt` format, as yt-dlp exports them from a browser. The
//! sign-in is kept in that format too, so an export can be used as it is.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    /// With a leading dot when it also applies to subdomains, as browsers write it.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Seconds since the epoch; 0 for a cookie that only lasts the session.
    pub expires: u64,
    pub name: String,
    pub value: String,
}

impl Cookie {
    fn matches(&self, host: &str) -> bool {
        let domain = self.domain.trim_start_matches('.');
        host == domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
    }

    fn expired(&self, now: u64) -> bool {
        self.expires != 0 && self.expires <= now
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Jar {
    cookies: Vec<Cookie>,
}

impl Jar {
    /// Reads a `cookies.txt`. Lines it doesn't understand are skipped.
    pub fn parse(text: &str) -> Self {
        let cookies = text
            .lines()
            .filter_map(|line| {
                let (line, http_only) = match line.strip_prefix("#HttpOnly_") {
                    Some(rest) => (rest, true),
                    None if line.starts_with('#') => return None,
                    None => (line, false),
                };
                let fields: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
                let [domain, _subdomains, path, secure, expires, name, value] = fields[..] else { return None };
                Some(Cookie {
                    domain: domain.to_owned(),
                    path: path.to_owned(),
                    secure: secure.eq_ignore_ascii_case("TRUE"),
                    http_only,
                    expires: expires.parse().unwrap_or(0),
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
            })
            .collect();
        Self { cookies }
    }

    /// A `Cookie:` header as copied from the browser's developer tools, for youtube.com.
    pub fn from_header(header: &str) -> Self {
        let header = header.trim();
        let header = header.strip_prefix("Cookie:").or_else(|| header.strip_prefix("cookie:")).unwrap_or(header);
        let cookies = header
            .split(';')
            .filter_map(|pair| {
                let (name, value) = pair.trim().split_once('=')?;
                (!name.is_empty()).then(|| Cookie {
                    domain: ".youtube.com".into(),
                    path: "/".into(),
                    secure: true,
                    http_only: false,
                    expires: 0,
                    name: name.trim().to_owned(),
                    value: value.trim().to_owned(),
                })
            })
            .collect();
        Self { cookies }
    }

    /// The file as yt-dlp would write it.
    pub fn to_text(&self) -> String {
        let mut text = String::from("# Netscape HTTP Cookie File\n# Written by cloofy. Keep this file private.\n\n");
        for c in &self.cookies {
            let prefix = if c.http_only { "#HttpOnly_" } else { "" };
            let subdomains = if c.domain.starts_with('.') { "TRUE" } else { "FALSE" };
            let secure = if c.secure { "TRUE" } else { "FALSE" };
            text.push_str(&format!(
                "{prefix}{}\t{subdomains}\t{}\t{secure}\t{}\t{}\t{}\n",
                c.domain, c.path, c.expires, c.name, c.value
            ));
        }
        text
    }

    /// Only the cookies youtube.com gets; an export from a browser has every site's.
    pub fn youtube_only(self) -> Self {
        Self { cookies: self.cookies.into_iter().filter(|c| c.matches("music.youtube.com")).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// The value of a cookie that youtube.com gets.
    pub fn get(&self, name: &str) -> Option<&str> {
        let now = now();
        self.cookies
            .iter()
            .find(|c| c.name == name && c.matches("music.youtube.com") && !c.expired(now))
            .map(|c| c.value.as_str())
    }

    /// The `Cookie` header for requests to `host`.
    pub fn header(&self, host: &str) -> String {
        let now = now();
        self.cookies
            .iter()
            .filter(|c| c.matches(host) && !c.expired(now))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Applies a `Set-Cookie` header that came from `host`. Returns whether anything changed.
    pub fn update(&mut self, host: &str, set_cookie: &str) -> bool {
        let mut parts = set_cookie.split(';');
        let Some((name, value)) = parts.next().and_then(|pair| pair.split_once('=')) else { return false };
        let mut cookie = Cookie {
            domain: host.to_owned(),
            path: "/".into(),
            secure: false,
            http_only: false,
            expires: 0,
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
        };
        let mut max_age = None;
        for attribute in parts {
            let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
            match key.trim().to_ascii_lowercase().as_str() {
                "domain" => cookie.domain = format!(".{}", value.trim().trim_start_matches('.')),
                "path" => cookie.path = value.trim().to_owned(),
                "secure" => cookie.secure = true,
                "httponly" => cookie.http_only = true,
                "expires" => cookie.expires = parse_http_date(value).unwrap_or(0),
                "max-age" => max_age = value.trim().parse::<i64>().ok(),
                _ => {}
            }
        }
        if let Some(seconds) = max_age {
            cookie.expires = if seconds <= 0 { 1 } else { now() + seconds as u64 };
        }
        let position = self
            .cookies
            .iter()
            .position(|c| c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path);
        if cookie.expired(now()) {
            return position.map(|i| self.cookies.remove(i)).is_some();
        }
        match position {
            Some(i) if self.cookies[i] == cookie => false,
            Some(i) => {
                self.cookies[i] = cookie;
                true
            }
            None => {
                self.cookies.push(cookie);
                true
            }
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Parses dates like `Wed, 27-Oct-2027 19:00:00 GMT` or `Wed, 27 Oct 2027 19:00:00 GMT`.
fn parse_http_date(text: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let (mut day, mut month, mut year, mut time) = (None, None, None, None);
    for token in text.split([' ', '-', ',']).filter(|t| !t.is_empty()) {
        if token.contains(':') {
            let mut hms = token.split(':').map(|n| n.parse::<u64>().ok());
            time = Some((hms.next()??, hms.next()??, hms.next().flatten().unwrap_or(0)));
        } else if let Ok(n) = token.parse::<u64>() {
            if n > 31 { year = Some(if n < 100 { n + 2000 } else { n }) } else { day = Some(n) }
        } else if let Some(m) = MONTHS.iter().position(|m| token.len() >= 3 && token[..3].eq_ignore_ascii_case(m)) {
            month = Some(m as u64 + 1);
        }
    }
    let (h, m, s) = time.unwrap_or((0, 0, 0));
    Some(days_from_civil(year?, month?, day?) * 86_400 + h * 3600 + m * 60 + s)
}

/// Days since 1970-01-01 of a date in the Gregorian calendar (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after 1970-01-01, as (year, month, day).
pub(crate) fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Netscape HTTP Cookie File\n\
        .youtube.com\tTRUE\t/\tTRUE\t1893456000\tSAPISID\tabc/def\n\
        #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1893456000\tLOGIN_INFO\tlogin\n\
        .google.com\tTRUE\t/\tTRUE\t1893456000\tSID\tgoogle\n\
        .youtube.com\tTRUE\t/\tTRUE\t1000\tOLD\tgone\n";

    #[test]
    fn reads_and_writes_a_cookies_file() {
        let jar = Jar::parse(FILE);
        assert_eq!(jar.get("SAPISID"), Some("abc/def"));
        assert_eq!(jar.get("LOGIN_INFO"), Some("login"));
        assert_eq!(jar.get("OLD"), None, "expired cookies are left out");
        assert_eq!(Jar::parse(&jar.to_text()), jar);
    }

    #[test]
    fn sends_only_the_cookies_of_the_host() {
        let jar = Jar::parse(FILE);
        assert_eq!(jar.header("music.youtube.com"), "SAPISID=abc/def; LOGIN_INFO=login");
        assert_eq!(jar.header("accounts.google.com"), "SID=google");
        assert_eq!(jar.header("notyoutube.com"), "");
    }

    #[test]
    fn takes_a_pasted_cookie_header() {
        let jar = Jar::from_header("Cookie: SAPISID=abc; LOGIN_INFO=x=y; empty=");
        assert_eq!(jar.get("SAPISID"), Some("abc"));
        assert_eq!(jar.get("LOGIN_INFO"), Some("x=y"));
        assert_eq!(jar.header("music.youtube.com"), "SAPISID=abc; LOGIN_INFO=x=y; empty=");
    }

    #[test]
    fn set_cookie_replaces_adds_and_removes() {
        let mut jar = Jar::parse(FILE);
        let header = "SAPISID=new; expires=Wed, 27-Oct-2077 19:00:00 GMT; path=/; domain=.youtube.com; Secure";
        assert!(jar.update("music.youtube.com", header));
        assert!(!jar.update("music.youtube.com", header), "the same value again changes nothing");
        assert_eq!(jar.get("SAPISID"), Some("new"));
        assert!(jar.update("music.youtube.com", "YSC=1; path=/; domain=youtube.com; HttpOnly"));
        assert_eq!(jar.get("YSC"), Some("1"));
        assert!(jar.update("music.youtube.com", "YSC=; Max-Age=0; path=/; domain=.youtube.com"));
        assert_eq!(jar.get("YSC"), None);
    }

    #[test]
    fn parses_cookie_dates() {
        assert_eq!(parse_http_date("Wed, 27-Oct-2027 19:00:00 GMT"), Some(1_824_663_600));
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:10 GMT"), Some(10));
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 27)), (2026, 10, 27));
    }
}

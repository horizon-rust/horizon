//! Microsoft account sign-in: the OAuth device-code flow, then Xbox Live, XSTS and the
//! Minecraft services, the ownership check and the profile. The same steps as Horizon's
//! desktop launcher (launcher/src/account.rs), without its keychain and accounts file.
//!
//! The Azure app (client ID) is configuration (see [`crate::store::client_id`]). New apps
//! are refused by the Minecraft services until Mojang approves them.

use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCOPE: &str = "XboxLive.signin offline_access";
/// The live.com flow's scope, as the Minecraft apps ask for it.
const LIVE_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";

/// Which sign-in an app ID uses: an Azure application ID (a UUID) goes through the Microsoft
/// identity platform; a 16-digit hex ID is one of the Minecraft apps' own (the way
/// `prismarine-auth` and other bots and launchers sign in) and goes through live.com.
pub fn is_live_id(client_id: &str) -> bool {
    client_id.len() == 16 && client_id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The services' addresses: the real ones, or a mock in tests.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub microsoft: String,
    pub xbox_user: String,
    pub xsts: String,
    pub minecraft: String,
    /// The older live.com device-code endpoints the Minecraft apps' own IDs use.
    pub live_device: String,
    pub live_token: String,
}

impl Endpoints {
    pub fn live() -> Self {
        Self {
            microsoft: "https://login.microsoftonline.com/consumers/oauth2/v2.0".into(),
            xbox_user: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft: "https://api.minecraftservices.com".into(),
            live_device: "https://login.live.com/oauth20_connect.srf".into(),
            live_token: "https://login.live.com/oauth20_token.srf".into(),
        }
    }
}

enum Body<'a> {
    Form(Vec<(&'a str, &'a str)>),
    Json(Value),
}

fn request(method: &str, url: &str) -> ureq::Request {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("horizon-mobile")
        // The player's proxy for the real services; never for a plain-http address (a test's).
        .try_proxy_from_env(url.starts_with("https://"))
        .build()
        .request(method, url)
}

/// One request's status and JSON body (Null when there is none). Transport errors name the
/// service, never the URL or the request.
fn send(req: ureq::Request, body: Option<Body>, service: &str) -> Result<(u16, Value), String> {
    let result = match body {
        None => req.call(),
        Some(Body::Form(fields)) => req.send_form(&fields),
        Some(Body::Json(v)) => req.set("Accept", "application/json").send_json(v),
    };
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(format!(
                "Could not reach {service}. Check your internet connection."
            ));
        }
    };
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// `send`, and the first `Set-Cookie` of the answer as `name=value`.
fn send_cookie(
    req: ureq::Request,
    body: Body,
    service: &str,
) -> Result<(u16, Value, Option<String>), String> {
    let result = match body {
        Body::Form(fields) => req.send_form(&fields),
        Body::Json(v) => req.set("Accept", "application/json").send_json(v),
    };
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(format!(
                "Could not reach {service}. Check your internet connection."
            ));
        }
    };
    let status = response.status();
    let cookie = response
        .header("set-cookie")
        .and_then(|c| c.split(';').next())
        .map(str::to_owned);
    let text = response.into_string().unwrap_or_default();
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::Null),
        cookie,
    ))
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the player types at the verification page.
#[derive(Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_at: u64,
    device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish_non_exhaustive()
    }
}

/// The Microsoft tokens; the access token only lasts for the Xbox step.
pub struct MsTokens {
    access: String,
    pub refresh: String,
    /// Signed in through live.com (the Xbox ticket is `t=`, not `d=`).
    live: bool,
}

impl std::fmt::Debug for MsTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MsTokens(<hidden>)")
    }
}

pub enum Poll {
    /// Not signed in yet, with Microsoft's own words for it.
    Pending(String),
    SlowDown,
    Done(MsTokens),
}

pub fn device_code(ep: &Endpoints, client_id: &str) -> Result<DeviceCode, String> {
    let live = is_live_id(client_id);
    let (url, fields) = if live {
        (
            ep.live_device.clone(),
            vec![
                ("client_id", client_id),
                ("scope", LIVE_SCOPE),
                ("response_type", "device_code"),
            ],
        )
    } else {
        (
            format!("{}/devicecode", ep.microsoft),
            vec![("client_id", client_id), ("scope", SCOPE)],
        )
    };
    let (status, v, _cookie) = send_cookie(request("POST", &url), Body::Form(fields), "Microsoft")?;
    match (
        status,
        str_field(&v, "user_code"),
        str_field(&v, "device_code"),
    ) {
        (200, Some(user_code), Some(device_code)) => Ok(DeviceCode {
            user_code,
            verification_uri: str_field(&v, "verification_uri")
                .unwrap_or_else(|| "https://www.microsoft.com/link".into()),
            interval: v
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .max(1),
            expires_at: now() + v.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
            device_code,
        }),
        _ => Err(oauth_error(&v)),
    }
}

fn oauth_error(v: &Value) -> String {
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_declined") => "Sign-in was declined in the browser.".into(),
        Some("expired_token") => "The sign-in code expired. Start again.".into(),
        Some("bad_verification_code") => "The sign-in code was not recognised. Start again.".into(),
        Some("invalid_grant") => "Your Microsoft sign-in expired. Sign in again.".into(),
        Some("invalid_client" | "unauthorized_client") => {
            "Microsoft refused this app's registration (client ID).".into()
        }
        Some(other) => format!("Microsoft sign-in failed ({other})."),
        None => "Microsoft sign-in failed.".into(),
    }
}

/// What a live.com poll answer means. live.com answers "invalid_grant" while the code has not
/// been entered yet as well as after a real failure, so only the fatal codes end the wait.
#[derive(Debug, PartialEq, Eq)]
pub enum LiveAnswer {
    Pending,
    SlowDown,
    Fatal,
}

/// What a "not yet" answer says, short, for the screen.
pub fn pending_text(v: &Value) -> String {
    let error = v.get("error").and_then(Value::as_str).unwrap_or("pending");
    let detail = v
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let detail = detail.split(['\r', '\n']).next().unwrap_or("");
    let mut text = format!("waiting ({error}) {detail}");
    text.truncate(110);
    text
}

pub fn live_answer(v: &Value) -> LiveAnswer {
    match v.get("error").and_then(Value::as_str) {
        Some("slow_down") => LiveAnswer::SlowDown,
        Some("expired_token" | "access_denied" | "authorization_declined") => LiveAnswer::Fatal,
        _ => LiveAnswer::Pending,
    }
}

pub fn poll(ep: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Poll, String> {
    if is_live_id(client_id) {
        // Exactly the poll the EDU-DONUT bot's msa-patch.js sends (and MinecraftAuth): the plain
        // token endpoint, and the grant type "device_code".
        let (status, v) = send(
            request("POST", &ep.live_token),
            Some(Body::Form(vec![
                ("client_id", client_id),
                ("device_code", &code.device_code),
                ("grant_type", "device_code"),
            ])),
            "Microsoft",
        )?;
        if v.get("error").is_some() {
            return match live_answer(&v) {
                LiveAnswer::Pending => Ok(Poll::Pending(pending_text(&v))),
                LiveAnswer::SlowDown => Ok(Poll::SlowDown),
                LiveAnswer::Fatal => Err(oauth_error(&v)),
            };
        }
        return ms_tokens(status, &v, true).map(Poll::Done);
    }
    let (status, v) = send(
        request("POST", &format!("{}/token", ep.microsoft)),
        Some(Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", &code.device_code),
        ])),
        "Microsoft",
    )?;
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => {
            return Ok(Poll::Pending("waiting for you to enter the code".into()));
        }
        Some("slow_down") => return Ok(Poll::SlowDown),
        _ => {}
    }
    ms_tokens(status, &v, false).map(Poll::Done)
}

fn ms_tokens(status: u16, v: &Value, live: bool) -> Result<MsTokens, String> {
    match (
        status,
        str_field(v, "access_token"),
        str_field(v, "refresh_token"),
    ) {
        (200, Some(access), Some(refresh)) => Ok(MsTokens {
            access,
            refresh,
            live,
        }),
        _ => Err(oauth_error(v)),
    }
}

/// Waits for the player to finish in the browser, polling as asked, until the code expires or
/// `cancelled` says stop.
pub fn wait_for_sign_in(
    ep: &Endpoints,
    client_id: &str,
    code: &DeviceCode,
    cancelled: &dyn Fn() -> bool,
    status: &dyn Fn(&str),
) -> Result<MsTokens, String> {
    // The bot waits at least five seconds between polls.
    let mut interval = code.interval.max(5);
    let mut polls = 0u32;
    loop {
        let until = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err("Sign-in cancelled.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if now() > code.expires_at {
            return Err("The sign-in code expired. Start again.".into());
        }
        polls += 1;
        match poll(ep, client_id, code) {
            Ok(Poll::Pending(text)) => status(&format!("{text} (check {polls})")),
            Ok(Poll::SlowDown) => {
                interval += 5;
                status("Microsoft asked to slow down");
            }
            Ok(Poll::Done(tokens)) => return Ok(tokens),
            // A network hiccup (the app was in the background) is not the end of the sign-in.
            Err(e) if e.starts_with("Could not reach") => status(&e),
            Err(e) => return Err(e),
        }
    }
}

pub fn refresh(ep: &Endpoints, client_id: &str, refresh_token: &str) -> Result<MsTokens, String> {
    let live = is_live_id(client_id);
    let (url, scope) = if live {
        (ep.live_token.clone(), LIVE_SCOPE)
    } else {
        (format!("{}/token", ep.microsoft), SCOPE)
    };
    let (status, v) = send(
        request("POST", &url),
        Some(Body::Form(vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", scope),
        ])),
        "Microsoft",
    )?;
    ms_tokens(status, &v, live)
}

fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => "This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one, then try again.".into(),
        2148916235 => "Xbox Live is not available in your country or region.".into(),
        2148916236 | 2148916237 => "This account needs adult verification on xbox.com before it can play.".into(),
        2148916238 => "This is a child account: an adult has to add it to a Microsoft family first.".into(),
        other => format!("Xbox Live refused the sign-in (error {other})."),
    }
}

/// The Minecraft access token from the Microsoft one, through Xbox Live and XSTS.
fn minecraft_login(ep: &Endpoints, ms_access: &str, live: bool) -> Result<String, String> {
    let (status, xbl) = send(
        request("POST", &ep.xbox_user),
        Some(Body::Json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("{}{ms_access}", if live { "t=" } else { "d=" }),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")
        .filter(|_| status == 200)
        .ok_or("Xbox Live refused the sign-in.")?;
    let (status, xsts) = send(
        request("POST", &ep.xsts),
        Some(Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    if let Some(code) = xsts.get("XErr").and_then(Value::as_u64) {
        return Err(xsts_error(code));
    }
    let uhs = xsts
        .pointer("/DisplayClaims/xui/0")
        .and_then(|c| str_field(c, "uhs"));
    let (Some(xsts_token), Some(uhs)) = (str_field(&xsts, "Token").filter(|_| status == 200), uhs)
    else {
        return Err("Xbox Live refused the sign-in.".into());
    };
    let identity = format!("XBL3.0 x={uhs};{xsts_token}");
    let (status, mc) = send(
        request(
            "POST",
            &format!("{}/authentication/login_with_xbox", ep.minecraft),
        ),
        Some(Body::Json(json!({ "identityToken": identity }))),
        "the Minecraft services",
    )?;
    match (status, str_field(&mc, "access_token")) {
        (200, Some(token)) => Ok(token),
        (403, _) => Err(
            "The Minecraft services refused this app: its Azure app is not approved by Mojang yet."
                .into(),
        ),
        (429, _) => Err("Too many sign-ins in a short time. Wait a minute and try again.".into()),
        _ => Err("The Minecraft services refused the sign-in.".into()),
    }
}

fn bearer(req: ureq::Request, token: &str) -> ureq::Request {
    req.set("Authorization", &format!("Bearer {token}"))
}

/// Whether the entitlements list has Minecraft: Java Edition (Game Pass included).
pub fn has_game(entitlements: &Value) -> bool {
    entitlements
        .get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("name").and_then(Value::as_str),
                    Some("game_minecraft" | "product_minecraft")
                )
            })
        })
}

/// A verified owner of Minecraft: Java Edition.
#[derive(Clone, PartialEq, Eq)]
pub struct Owner {
    pub name: String,
    pub uuid: String,
    /// The Microsoft refresh token to keep for next time.
    pub refresh_token: String,
    /// The Minecraft access token (about a day), for the session the game joins online servers with.
    pub minecraft_token: String,
}

impl std::fmt::Debug for Owner {
    // The tokens are secrets: never in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Owner")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .finish_non_exhaustive()
    }
}

/// Turns Microsoft tokens into a verified owner: Xbox Live, XSTS, the Minecraft token, the
/// entitlements check and the profile.
pub fn verify_owner(ep: &Endpoints, tokens: MsTokens) -> Result<Owner, String> {
    let token = minecraft_login(ep, &tokens.access, tokens.live)?;
    let (status, ent) = send(
        bearer(
            request("GET", &format!("{}/entitlements/mcstore", ep.minecraft)),
            &token,
        ),
        None,
        "the Minecraft services",
    )?;
    if status != 200 {
        return Err("Could not check which games this account owns.".into());
    }
    if !has_game(&ent) {
        return Err("This account doesn't own Minecraft: Java Edition.".into());
    }
    let (status, profile) = send(
        bearer(
            request("GET", &format!("{}/minecraft/profile", ep.minecraft)),
            &token,
        ),
        None,
        "the Minecraft services",
    )?;
    match (status, str_field(&profile, "id"), str_field(&profile, "name")) {
        (200, Some(uuid), Some(name)) => Ok(Owner { name, uuid, refresh_token: tokens.refresh, minecraft_token: token.clone() }),
        (404, _, _) => Err(
            "This account has no Minecraft profile yet. Create one at minecraft.net, then sign in again."
                .into(),
        ),
        _ => Err("Could not read this account's Minecraft profile.".into()),
    }
}

/// Signs in again from a saved refresh token and re-checks ownership.
pub fn verify_saved(ep: &Endpoints, client_id: &str, refresh_token: &str) -> Result<Owner, String> {
    verify_owner(ep, refresh(ep, client_id, refresh_token)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_needs_a_java_entitlement() {
        assert!(has_game(&json!({"items":[{"name":"game_minecraft"}]})));
        assert!(has_game(
            &json!({"items":[{"name":"x"},{"name":"product_minecraft"}]})
        ));
        assert!(!has_game(
            &json!({"items":[{"name":"game_minecraft_bedrock"}]})
        ));
        assert!(!has_game(&json!({"items":[]})));
        assert!(!has_game(&Value::Null));
    }

    #[test]
    fn oauth_errors_are_readable() {
        assert!(oauth_error(&json!({"error":"authorization_declined"})).contains("declined"));
        assert!(oauth_error(&json!({"error":"weird"})).contains("weird"));
        assert!(oauth_error(&Value::Null).contains("failed"));
    }

    #[test]
    fn app_ids_pick_their_flow() {
        assert!(is_live_id("00000000402b5328"));
        assert!(!is_live_id("12345678-abcd-1234-abcd-1234567890ab"));
        assert!(!is_live_id("0000000402b5328"));
    }

    #[test]
    fn live_answers_keep_polling_except_for_fatal_codes() {
        assert_eq!(
            live_answer(&json!({"error":"authorization_pending"})),
            LiveAnswer::Pending
        );
        // live.com says this until the code is entered.
        assert_eq!(
            live_answer(&json!({"error":"invalid_grant"})),
            LiveAnswer::Pending
        );
        assert_eq!(
            live_answer(&json!({"error":"slow_down"})),
            LiveAnswer::SlowDown
        );
        assert_eq!(
            live_answer(&json!({"error":"expired_token"})),
            LiveAnswer::Fatal
        );
        assert_eq!(
            live_answer(&json!({"error":"authorization_declined"})),
            LiveAnswer::Fatal
        );
    }

    #[test]
    fn pending_text_is_short_and_one_line() {
        let t = pending_text(
            &json!({"error":"invalid_grant","error_description":"first line\r\nTrace ID: x"}),
        );
        assert_eq!(t, "waiting (invalid_grant) first line");
        assert!(
            pending_text(&json!({"error":"x","error_description":"y".repeat(500)})).len() <= 110
        );
    }

    #[test]
    fn debug_never_shows_tokens() {
        let t = MsTokens {
            access: "A".into(),
            refresh: "R".into(),
            live: false,
        };
        assert_eq!(format!("{t:?}"), "MsTokens(<hidden>)");
    }

    /// A tiny local HTTP server: answers each request by its path from `routes` (path prefix,
    /// status, extra headers, body) and records every request's text.
    fn mock(
        routes: Vec<(&'static str, u16, &'static str, Vec<&'static str>)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let mut counts = std::collections::HashMap::<&str, usize>::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = vec![0u8; 16384];
                let mut n = 0;
                // Read until the headers and the declared body have arrived.
                loop {
                    let got = stream.read(&mut buf[n..]).unwrap_or(0);
                    if got == 0 {
                        break;
                    }
                    n += got;
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if n >= i + 4 + len {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                log.lock().unwrap().push(request);
                // The k-th request to a path takes the k-th route registered for it.
                let matching: Vec<_> = routes.iter().filter(|r| path.starts_with(r.0)).collect();
                let count = counts
                    .entry(matching.first().map_or("", |r| r.0))
                    .or_insert(0);
                let route = matching.get(*count).or(matching.last()).copied();
                *count += 1;
                let (status, headers, body) =
                    route.map_or((404, "", vec!["{}"]), |r| (r.1, r.2, r.3.clone()));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body[0].len(),
                    body[0]
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, seen)
    }

    #[test]
    fn the_live_flow_end_to_end_against_a_mock() {
        let (base, seen) = mock(vec![
            (
                "/connect",
                200,
                "Set-Cookie: MSPRequ=abc; path=/\r\n",
                vec![
                    r#"{"user_code":"ABCD1234","device_code":"dev","verification_uri":"https://www.microsoft.com/link","interval":1,"expires_in":900}"#,
                ],
            ),
            (
                "/token",
                400,
                "",
                vec![r#"{"error":"invalid_grant","error_description":"not yet"}"#],
            ),
            (
                "/token",
                200,
                "",
                vec![r#"{"access_token":"LIVEACCESS","refresh_token":"LIVEREFRESH"}"#],
            ),
            ("/xbl", 200, "", vec![r#"{"Token":"XBLTOKEN"}"#]),
            (
                "/xsts",
                200,
                "",
                vec![r#"{"Token":"XSTSTOKEN","DisplayClaims":{"xui":[{"uhs":"UHS"}]}}"#],
            ),
            (
                "/authentication/login_with_xbox",
                200,
                "",
                vec![r#"{"access_token":"MCTOKEN"}"#],
            ),
            (
                "/entitlements/mcstore",
                200,
                "",
                vec![r#"{"items":[{"name":"game_minecraft"}]}"#],
            ),
            (
                "/minecraft/profile",
                200,
                "",
                vec![r#"{"id":"0123456789abcdef0123456789abcdef","name":"Steve"}"#],
            ),
        ]);
        let ep = Endpoints {
            microsoft: format!("{base}/ms"),
            xbox_user: format!("{base}/xbl"),
            xsts: format!("{base}/xsts"),
            minecraft: base.clone(),
            live_device: format!("{base}/connect"),
            live_token: format!("{base}/token"),
        };
        let id = "00000000402b5328";
        let code = device_code(&ep, id).unwrap();
        assert_eq!(code.user_code, "ABCD1234");
        // The first poll is "invalid_grant": still waiting, not a failure.
        assert!(matches!(poll(&ep, id, &code).unwrap(), Poll::Pending(_)));
        let tokens = match poll(&ep, id, &code).unwrap() {
            Poll::Done(t) => t,
            _ => panic!("expected tokens"),
        };
        let owner = verify_owner(&ep, tokens).unwrap();
        assert_eq!(
            owner,
            Owner {
                name: "Steve".into(),
                uuid: "0123456789abcdef0123456789abcdef".into(),
                refresh_token: "LIVEREFRESH".into(),
                minecraft_token: "MCTOKEN".into()
            }
        );
        assert!(!format!("{owner:?}").contains("MCTOKEN"));

        let requests = seen.lock().unwrap().clone();
        let find = |needle: &str| {
            requests
                .iter()
                .find(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no request with {needle}"))
                .clone()
        };
        // The code request names the app, the live scope and the device-code response type.
        let first = find("POST /connect");
        assert!(first.contains("client_id=00000000402b5328"));
        assert!(first.contains("response_type=device_code"));
        assert!(first.contains("MBI_SSL"));
        // The polls are the bot's: plain token endpoint, grant type "device_code".
        let poll_req = find("POST /token");
        assert!(poll_req.contains("grant_type=device_code"));
        assert!(poll_req.contains("device_code=dev"));
        assert!(!poll_req.contains("urn%3Aietf"));
        // Xbox gets the live ticket form, `t=`.
        assert!(find("POST /xbl").contains(r#""RpsTicket":"t=LIVEACCESS""#));
        assert!(find("POST /xsts").contains("rp://api.minecraftservices.com/"));
        assert!(find("POST /authentication/login_with_xbox").contains("XBL3.0 x=UHS;XSTSTOKEN"));
        assert!(
            find("GET /entitlements/mcstore")
                .to_ascii_lowercase()
                .contains("authorization: bearer mctoken")
        );
    }

    #[test]
    fn a_non_owner_is_refused() {
        let (base, _) = mock(vec![
            ("/xbl", 200, "", vec![r#"{"Token":"X"}"#]),
            (
                "/xsts",
                200,
                "",
                vec![r#"{"Token":"S","DisplayClaims":{"xui":[{"uhs":"U"}]}}"#],
            ),
            (
                "/authentication/login_with_xbox",
                200,
                "",
                vec![r#"{"access_token":"T"}"#],
            ),
            ("/entitlements/mcstore", 200, "", vec![r#"{"items":[]}"#]),
        ]);
        let ep = Endpoints {
            microsoft: base.clone(),
            xbox_user: format!("{base}/xbl"),
            xsts: format!("{base}/xsts"),
            minecraft: base.clone(),
            live_device: base.clone(),
            live_token: base.clone(),
        };
        let err = verify_owner(
            &ep,
            MsTokens {
                access: "a".into(),
                refresh: "r".into(),
                live: true,
            },
        )
        .unwrap_err();
        assert!(err.contains("doesn't own"), "{err}");
    }
}

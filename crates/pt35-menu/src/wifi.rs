//! Joining a Wi-Fi network: what a network needs, the password prompt, and
//! the nmcli run behind it. Only [`connect`] touches the system, so the rest
//! tests on any host.

use pt35_ui::keys::{button, sym, Button, Key, Mode};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

/// Longest WPA passphrase, and the 64 hex digits of a raw key.
const MAX_LEN: usize = 64;

/// What a network asks for, from nmcli's SECURITY column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA or WPA3 personal: one passphrase.
    Psk,
    Wep,
    /// 802.1X wants an identity too. It is joined in a terminal, where nmcli
    /// asks for each.
    Enterprise,
}

impl Security {
    pub fn from_nmcli(field: &str) -> Self {
        if field.contains("802.1X") {
            Security::Enterprise
        } else if field.contains("WEP") {
            Security::Wep
        } else if field.contains("WPA") || field.contains("SAE") {
            Security::Psk
        } else {
            // Empty, `--`, or OWE, which encrypts without a password.
            Security::Open
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Psk => "psk",
            Security::Wep => "wep",
            Security::Enterprise => "eap",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "open" => Security::Open,
            "psk" => Security::Psk,
            "wep" => Security::Wep,
            "eap" => Security::Enterprise,
            _ => return None,
        })
    }

    fn asks(self) -> bool {
        matches!(self, Security::Psk | Security::Wep)
    }
}

/// A row on the Network screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    pub security: Security,
    /// NetworkManager has a profile for it, with its password.
    pub saved: bool,
}

impl Network {
    /// `<security>:<saved|new>:<ssid>`. The SSID is last, so a `:` in it is
    /// safe.
    pub fn payload(&self) -> String {
        let saved = if self.saved { "saved" } else { "new" };
        format!("{}:{saved}:{}", self.security.tag(), self.ssid)
    }

    pub fn parse(payload: &str) -> Option<Self> {
        let mut parts = payload.splitn(3, ':');
        let security = Security::from_tag(parts.next()?)?;
        let saved = match parts.next()? {
            "saved" => true,
            "new" => false,
            _ => return None,
        };
        Some(Network {
            ssid: parts.next()?.to_string(),
            security,
            saved,
        })
    }

    /// The password is asked first only when nothing else can supply it.
    pub fn asks_first(&self) -> bool {
        !self.saved && self.security.asks()
    }
}

/// Why a join did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A saved password was missing or stale.
    NeedsPassword,
    WrongPassword,
    Other(String),
}

/// What a key means to the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Redraw,
    /// The password passed its check: start the join.
    Connect,
    Cancel,
    /// Start: close the menu, as everywhere.
    Quit,
    /// R: switch input mode, and keep the prompt.
    Pass,
    Nothing,
}

pub struct Prompt {
    pub network: Network,
    password: String,
    pub reveal: bool,
    pub connecting: bool,
    /// What to say under the field, and whether it is an error.
    pub message: Option<(String, bool)>,
}

impl Prompt {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            password: String::new(),
            reveal: false,
            connecting: false,
            message: None,
        }
    }

    /// A prompt for a network that needs no typing: it starts connecting.
    pub fn connecting(network: Network) -> Self {
        let mut prompt = Self::new(network);
        prompt.start();
        prompt
    }

    /// Whether there is a password to type. A saved network has one already,
    /// until NetworkManager says it does not work.
    pub fn has_field(&self) -> bool {
        !self.network.saved && self.network.security.asks()
    }

    /// The field as drawn: dots, or the text when revealed.
    pub fn shown(&self) -> String {
        if self.reveal {
            self.password.clone()
        } else {
            "\u{2022}".repeat(self.password.chars().count())
        }
    }

    /// A copy for the join, which wipes it when done.
    pub fn password(&self) -> Option<String> {
        self.has_field().then(|| self.password.clone())
    }

    pub fn key(&mut self, key: &Key) -> Action {
        // In filter mode letters are text, so on stock firmware the face
        // buttons type: a password may need an `a`. Enter, Esc and Tab do
        // their jobs there.
        match button(key, Mode::Filter) {
            Some(Button::Start) => return Action::Quit,
            Some(Button::R) => return Action::Pass,
            Some(Button::B) | Some(Button::Select) => return Action::Cancel,
            Some(Button::A) => return self.submit(),
            Some(Button::X) => return self.toggle_reveal(),
            Some(_) => return Action::Nothing,
            None => {}
        }
        match key.sym {
            sym::ESCAPE => return Action::Cancel,
            sym::RETURN | sym::KP_ENTER => return self.submit(),
            sym::TAB | sym::ISO_LEFT_TAB => return self.toggle_reveal(),
            _ => {}
        }
        if self.connecting || !self.has_field() {
            return Action::Nothing;
        }
        if key.sym == sym::BACKSPACE {
            return self.edit(|text| {
                text.pop();
            });
        }
        if key.ctrl && key.text.is_some_and(|c| c.eq_ignore_ascii_case(&'u')) {
            return self.edit(String::clear);
        }
        match key.text {
            Some(c) if !key.ctrl && (c.is_ascii_graphic() || c == ' ') => self.edit(|text| {
                if text.len() < MAX_LEN {
                    text.push(c);
                }
            }),
            _ => Action::Nothing,
        }
    }

    pub fn toggle_reveal(&mut self) -> Action {
        if !self.has_field() {
            return Action::Nothing;
        }
        self.reveal = !self.reveal;
        Action::Redraw
    }

    fn edit(&mut self, change: impl FnOnce(&mut String)) -> Action {
        change(&mut self.password);
        // An old "Wrong password" no longer describes what is in the field.
        if self.message.as_ref().is_some_and(|(_, error)| *error) {
            self.message = None;
        }
        Action::Redraw
    }

    fn submit(&mut self) -> Action {
        if self.connecting {
            return Action::Nothing;
        }
        if let Err(problem) = self.check() {
            self.message = Some((problem.into(), true));
            return Action::Redraw;
        }
        self.start();
        Action::Connect
    }

    fn start(&mut self) {
        self.connecting = true;
        self.message = Some(("Connecting...".into(), false));
    }

    /// What NetworkManager would refuse anyway, said before it is asked.
    pub fn check(&self) -> Result<(), &'static str> {
        let text = &self.password;
        match self.network.security {
            Security::Psk => {
                let raw_key = text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit());
                if (8..=63).contains(&text.len()) || raw_key {
                    Ok(())
                } else {
                    Err("8 to 63 characters")
                }
            }
            Security::Wep if text.is_empty() => Err("Type the password"),
            _ => Ok(()),
        }
    }

    /// The join came back with this.
    pub fn failed(&mut self, failure: Failure) {
        self.connecting = false;
        // A saved network whose password went stale is typed in like a new one.
        if matches!(failure, Failure::NeedsPassword | Failure::WrongPassword) {
            self.network.saved = false;
        }
        self.message = Some(match failure {
            Failure::NeedsPassword => ("Password needed".into(), false),
            Failure::WrongPassword => ("Wrong password".into(), true),
            Failure::Other(message) => (message, true),
        });
    }
}

impl Drop for Prompt {
    fn drop(&mut self) {
        wipe(&mut self.password);
    }
}

/// Overwrite a secret before its memory goes back to the allocator.
fn wipe(secret: &mut String) {
    let mut bytes = std::mem::take(secret).into_bytes();
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

/// Join on a worker thread. The answer arrives on the receiver.
pub fn connect(network: &Network, password: Option<String>) -> Receiver<Result<(), Failure>> {
    let (tx, rx) = mpsc::channel();
    let network = network.clone();
    std::thread::spawn(move || {
        let _ = tx.send(join(&network, password));
    });
    rx
}

fn join(network: &Network, mut password: Option<String>) -> Result<(), Failure> {
    let before = if network.saved {
        Vec::new()
    } else {
        wifi_profiles()
    };
    let gave = password.is_some();
    let result = run_nmcli(&network.ssid, password.as_deref());
    if let Some(secret) = password.as_mut() {
        wipe(secret);
    }
    let (ok, output) = result.map_err(Failure::Other)?;
    if ok {
        return Ok(());
    }
    // nmcli keeps the profile a failed first join made, wrong password and
    // all. The next scan would call it saved and never ask again.
    if !network.saved {
        forget_new(&network.ssid, &before);
    }
    Err(classify(&output, gave))
}

/// `nmcli --ask` reads the password from stdin, never from argv, where any
/// user could read it in `/proc`. With no password stdin closes empty, and a
/// prompt for one fails at once.
fn run_nmcli(ssid: &str, password: Option<&str>) -> Result<(bool, String), String> {
    let mut child = Command::new("nmcli")
        .args(["--wait", "45", "--ask", "device", "wifi", "connect", ssid])
        .env("LC_ALL", "C.UTF-8")
        .env_remove("LANGUAGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("nmcli: {e}"))?;
    if let (Some(mut stdin), Some(password)) = (child.stdin.take(), password) {
        let mut line = format!("{password}\n");
        let _ = stdin.write_all(line.as_bytes());
        wipe(&mut line);
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("nmcli: {e}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text))
}

pub fn classify(output: &str, gave_password: bool) -> Failure {
    let secrets = [
        "Secrets were required",
        "No agents were available",
        "psk: property is invalid",
    ];
    if secrets.iter().any(|s| output.contains(s)) {
        return if gave_password {
            Failure::WrongPassword
        } else {
            Failure::NeedsPassword
        };
    }
    let line = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Error: "))
        .unwrap_or("could not connect");
    let line = line
        .strip_prefix("Connection activation failed: ")
        .unwrap_or(line);
    Failure::Other(line.trim_end_matches('.').to_string())
}

/// UUIDs of the Wi-Fi profiles NetworkManager has.
pub fn wifi_profiles() -> Vec<String> {
    crate::providers::run("nmcli", &["-t", "-f", "UUID,TYPE", "connection", "show"])
        .map(|out| parse_profiles(&out))
        .unwrap_or_default()
}

pub fn parse_profiles(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(_, kind)| *kind == "802-11-wireless")
        .map(|(uuid, _)| uuid.to_string())
        .collect()
}

/// The SSIDs of those profiles, one nmcli call for all of them.
pub fn saved_ssids() -> Vec<String> {
    let uuids = wifi_profiles();
    if uuids.is_empty() {
        return Vec::new();
    }
    let mut args = vec!["-g", "802-11-wireless.ssid", "connection", "show"];
    args.extend(uuids.iter().map(String::as_str));
    crate::providers::run("nmcli", &args)
        .map(|out| parse_ssids(&out))
        .unwrap_or_default()
}

/// One value per profile, with blank lines between. nmcli escapes a `:` in
/// a value, or does not: joining the fields back reads both.
pub fn parse_ssids(out: &str) -> Vec<String> {
    out.lines()
        .filter(|line| !line.is_empty())
        .map(|line| crate::providers::nmcli_fields(line).join(":"))
        .collect()
}

fn forget_new(ssid: &str, before: &[String]) {
    for uuid in wifi_profiles() {
        if before.contains(&uuid) {
            continue;
        }
        let name = crate::providers::run(
            "nmcli",
            &["-g", "802-11-wireless.ssid", "connection", "show", &uuid],
        );
        if name.is_some_and(|n| parse_ssids(&n).first().map(String::as_str) == Some(ssid)) {
            let _ = crate::providers::run("nmcli", &["connection", "delete", &uuid]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(security: Security, saved: bool) -> Network {
        Network {
            ssid: "Cafe: 5G".into(),
            security,
            saved,
        }
    }

    fn typed(prompt: &mut Prompt, text: &str) {
        for c in text.chars() {
            prompt.key(&Key::with_text(c as u32, c));
        }
    }

    #[test]
    fn security_is_read_off_nmcli() {
        assert_eq!(Security::from_nmcli("WPA2"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA1 WPA2"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA3"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA2 802.1X"), Security::Enterprise);
        assert_eq!(Security::from_nmcli("WEP"), Security::Wep);
        assert_eq!(Security::from_nmcli(""), Security::Open);
        assert_eq!(Security::from_nmcli("OWE"), Security::Open);
    }

    #[test]
    fn a_payload_keeps_a_colon_in_the_name() {
        let net = network(Security::Psk, true);
        assert_eq!(net.payload(), "psk:saved:Cafe: 5G");
        assert_eq!(Network::parse(&net.payload()), Some(net));
        assert_eq!(Network::parse("screen:palette"), None);
        assert_eq!(Network::parse("psk:maybe:x"), None);
    }

    #[test]
    fn only_a_new_secured_network_asks_first() {
        assert!(network(Security::Psk, false).asks_first());
        assert!(network(Security::Wep, false).asks_first());
        assert!(!network(Security::Psk, true).asks_first());
        assert!(!network(Security::Open, false).asks_first());
    }

    #[test]
    fn typing_is_hidden_until_shown() {
        let mut prompt = Prompt::new(network(Security::Psk, false));
        typed(&mut prompt, "ab c");
        assert_eq!(prompt.shown(), "\u{2022}\u{2022}\u{2022}\u{2022}");
        assert_eq!(prompt.key(&Key::new(sym::TAB)), Action::Redraw);
        assert_eq!(prompt.shown(), "ab c");
        assert_eq!(prompt.key(&Key::new(sym::BUTTON_X)), Action::Redraw);
        assert!(!prompt.reveal);
    }

    #[test]
    fn letters_type_even_where_they_are_buttons() {
        pt35_ui::keys::set_letters_are_buttons(true);
        let mut prompt = Prompt::new(network(Security::Psk, false));
        typed(&mut prompt, "abxylr");
        prompt.reveal = true;
        assert_eq!(prompt.shown(), "abxylr");
    }

    #[test]
    fn backspace_and_ctrl_u_edit() {
        let mut prompt = Prompt::new(network(Security::Psk, false));
        prompt.reveal = true;
        typed(&mut prompt, "secret");
        prompt.key(&Key::new(sym::BACKSPACE));
        assert_eq!(prompt.shown(), "secre");
        let mut clear = Key::with_text('u' as u32, 'u');
        clear.ctrl = true;
        prompt.key(&clear);
        assert_eq!(prompt.shown(), "");
    }

    #[test]
    fn a_short_passphrase_is_caught_before_nmcli() {
        let mut prompt = Prompt::new(network(Security::Psk, false));
        typed(&mut prompt, "short");
        assert_eq!(prompt.key(&Key::new(sym::RETURN)), Action::Redraw);
        assert_eq!(prompt.message, Some(("8 to 63 characters".into(), true)));
        assert!(!prompt.connecting);
        typed(&mut prompt, "enough");
        assert_eq!(prompt.message, None, "typing clears an old error");
        assert_eq!(prompt.key(&Key::new(sym::BUTTON_A)), Action::Connect);
        assert!(prompt.connecting);
        assert_eq!(
            prompt.key(&Key::new(sym::RETURN)),
            Action::Nothing,
            "one join at a time"
        );
    }

    #[test]
    fn sixty_four_hex_digits_are_a_raw_key() {
        let mut prompt = Prompt::new(network(Security::Psk, false));
        typed(&mut prompt, &"a1".repeat(32));
        assert_eq!(prompt.check(), Ok(()));
        let mut prompt = Prompt::new(network(Security::Psk, false));
        typed(&mut prompt, &"g1".repeat(32));
        assert!(prompt.check().is_err());
    }

    #[test]
    fn escape_and_b_cancel_start_quits_r_passes() {
        let mut prompt = Prompt::new(network(Security::Psk, false));
        assert_eq!(prompt.key(&Key::new(sym::ESCAPE)), Action::Cancel);
        assert_eq!(prompt.key(&Key::new(sym::BUTTON_B)), Action::Cancel);
        assert_eq!(prompt.key(&Key::new(sym::BUTTON_START)), Action::Quit);
        assert_eq!(prompt.key(&Key::new(sym::BUTTON_R)), Action::Pass);
    }

    #[test]
    fn a_stale_saved_password_turns_into_a_field() {
        let mut prompt = Prompt::connecting(network(Security::Psk, true));
        assert!(prompt.connecting);
        assert!(!prompt.has_field(), "a saved network asks for nothing");
        assert_eq!(prompt.password(), None);
        prompt.failed(Failure::NeedsPassword);
        assert!(!prompt.connecting);
        assert!(!prompt.network.saved);
        assert_eq!(prompt.message, Some(("Password needed".into(), false)));
        assert!(prompt.has_field());
    }

    #[test]
    fn nmcli_errors_read_as_sentences() {
        let wrong = "Password: Error: Connection activation failed: Secrets were required, but not provided.\n";
        assert_eq!(classify(wrong, true), Failure::WrongPassword);
        assert_eq!(classify(wrong, false), Failure::NeedsPassword);
        assert_eq!(
            classify("Error: No network with SSID 'x' found.\n", false),
            Failure::Other("No network with SSID 'x' found".into())
        );
        assert_eq!(
            classify(
                "Error: Connection activation failed: (1) Unknown error.\n",
                true
            ),
            Failure::Other("(1) Unknown error".into())
        );
        assert_eq!(
            classify("", true),
            Failure::Other("could not connect".into())
        );
    }

    #[test]
    fn saved_profiles_and_their_names_are_read() {
        let profiles = "a1:802-11-wireless\nb2:loopback\nc3:802-11-wireless\n";
        assert_eq!(parse_profiles(profiles), ["a1", "c3"]);
        assert_eq!(
            parse_ssids("Mostly Harmless\n\nCafe\\: 5G\n\nraw:colon\n"),
            ["Mostly Harmless", "Cafe: 5G", "raw:colon"]
        );
    }
}

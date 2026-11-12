//! Which account a sign-in acts as. A browser's YouTube cookies sign in every Google account
//! signed in to that browser at once; YouTube tells them apart by their index among them
//! (`X-Goog-AuthUser`), and a brand account by its page ID (`X-Goog-PageId`). Signing in picks
//! one, and the sign-in keeps it along with the cookies.

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    innertube::InnerTube,
    parse::{self, find, find_all, str_at, text},
};

/// Google lets this many accounts be signed in to a browser at once.
const MAX_ACCOUNTS: u32 = 10;

/// The comments in the sign-in's `cookies.txt` that say which account it acts as.
const INDEX_NOTE: &str = "# cloofy-authuser: ";
const PAGE_NOTE: &str = "# cloofy-pageid: ";
const BROWSER_NOTE: &str = "# cloofy-browser: ";
const USER_NOTE: &str = "# cloofy-user: ";

/// The account requests act as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    /// The Google account's index among those signed in to the browser; 0 is the one signed
    /// in first.
    pub index: u32,
    /// A brand account's page ID; `None` for the Google account's own.
    pub page_id: Option<String>,
}

impl Identity {
    /// Reads the account from the sign-in's `cookies.txt`; the first one where it names none.
    pub(crate) fn parse(text: &str) -> Self {
        let mut identity = Self::default();
        for line in text.lines().map(|line| line.trim_end_matches('\r')) {
            if let Some(index) = line.strip_prefix(INDEX_NOTE).and_then(|index| index.trim().parse().ok()) {
                identity.index = index;
            } else if let Some(page) = line.strip_prefix(PAGE_NOTE).map(str::trim).filter(|page| !page.is_empty()) {
                identity.page_id = Some(page.to_owned());
            }
        }
        identity
    }

    /// The lines that keep it in a `cookies.txt`, as comments other readers skip.
    pub(crate) fn notes(&self) -> String {
        let mut notes = format!("{INDEX_NOTE}{}\n", self.index);
        if let Some(page) = &self.page_id {
            notes.push_str(&format!("{PAGE_NOTE}{page}\n"));
        }
        notes
    }
}

/// Where a sign-in came from, kept with it so that cloofy can read it from there again when
/// YouTube stops taking the cookies: a browser that goes on using its sign-in gets new cookies
/// for it, and the ones cloofy copied stop working after a day or two.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Origin {
    /// The browser as yt-dlp's `--cookies-from-browser` takes it; `None` for a pasted header.
    pub browser: Option<String>,
    /// YouTube's ID of the account signed in as. Cookies read again are only kept when they
    /// are this account's: the browser may have signed in to another one since.
    pub user: Option<String>,
}

impl Origin {
    /// Reads it from the sign-in's `cookies.txt`; empty where the file says nothing, as files
    /// from before cloofy 0.5.3 do.
    pub(crate) fn parse(text: &str) -> Self {
        let note = |prefix: &str| {
            let lines = text.lines().map(|line| line.trim_end_matches('\r'));
            lines.filter_map(|line| line.strip_prefix(prefix)).map(str::trim).rfind(|value| !value.is_empty())
        };
        Self { browser: note(BROWSER_NOTE).map(str::to_owned), user: note(USER_NOTE).map(str::to_owned) }
    }

    /// The lines that keep it in a `cookies.txt`, as comments other readers skip.
    pub(crate) fn notes(&self) -> String {
        let note = |prefix: &str, value: &Option<String>| value.as_ref().map(|value| format!("{prefix}{value}\n"));
        [note(BROWSER_NOTE, &self.browser), note(USER_NOTE, &self.user)].into_iter().flatten().collect()
    }
}

/// An account the cookies can act as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub identity: Identity,
    /// The name YouTube shows, which a work and a private account may share.
    pub name: String,
    /// The Google account's address; a brand account's is that of the Google account it
    /// belongs to.
    pub email: Option<String>,
    /// The channel's handle, such as `@name`.
    pub handle: Option<String>,
    /// What tells Google accounts apart: the address, else Google's ID of the account.
    google: String,
}

impl Account {
    /// The name with what tells it apart: "Ada · ada@example.com", or for a brand account
    /// "Band · brand account of ada@example.com".
    pub fn label(&self) -> String {
        let of = self.email.as_deref().or(self.handle.as_deref());
        match (of, &self.identity.page_id) {
            (Some(email), None) => format!("{} · {email}", self.name),
            (Some(email), Some(_)) => format!("{} · brand account of {email}", self.name),
            (None, _) => self.name.clone(),
        }
    }

    /// Whether it is the same account, wherever it is among the browser's.
    pub fn same_as(&self, other: &Account) -> bool {
        self.google == other.google && self.identity.page_id == other.identity.page_id
    }
}

/// Every account the cookies can act as: each Google account signed in to the browser, in
/// Google's order, each followed by its brand accounts. YouTube lists the accounts of one
/// index at a time, and an index past the last one answers as if signed out.
pub(crate) async fn list(api: &InnerTube) -> Result<Vec<Account>> {
    let mut accounts: Vec<Account> = Vec::new();
    for index in 0..MAX_ACCOUNTS {
        let identity = Identity { index, page_id: None };
        let page = match api.call_as_identity(&identity, "account/accounts_list", json!({})).await {
            Ok(page) => page,
            // The first answers for the cookies as a whole.
            Err(err) if index == 0 => return Err(err),
            Err(err) => {
                tracing::debug!("account {index}: {err:#}");
                break;
            }
        };
        let found = parse(&page, index);
        let known = found.first().is_none_or(|first| accounts.iter().any(|account| account.google == first.google));
        if known {
            break;
        }
        accounts.extend(found);
    }
    Ok(accounts)
}

/// The account the sign-in acts as, as `list` shows it.
pub(crate) async fn current(api: &InnerTube) -> Result<Option<Account>> {
    let Some(identity) = api.identity() else { return Ok(None) };
    let page = api.call("account/accounts_list", json!({})).await?;
    let accounts = parse(&page, identity.index);
    Ok(accounts.into_iter().find(|account| account.identity == identity))
}

/// The accounts on a page of `account/accounts_list`, as those of the Google account at
/// `index`. Disabled ones can't be used, so they are left out.
fn parse(page: &Value, index: u32) -> Vec<Account> {
    let mut sections = Vec::new();
    find_all(page, "accountSectionListRenderer", &mut sections);
    let mut accounts = Vec::new();
    for section in sections {
        let email = find(&section["header"], "googleAccountHeaderRenderer")
            .map(|header| text(&header["email"]))
            .filter(|email| !email.is_empty());
        let mut items = Vec::new();
        find_all(&section["contents"], "accountItem", &mut items);
        let mut google = email.clone();
        for item in items {
            let tokens = item.pointer("/serviceEndpoint/selectActiveIdentityEndpoint/supportedTokens");
            let tokens = tokens.map_or(&[][..], parse::list);
            let page_id = tokens.iter().find_map(|token| str_at(token, "/pageIdToken/pageId")).map(str::to_owned);
            let gaia = tokens.iter().find_map(|token| str_at(token, "/accountStateToken/obfuscatedGaiaId"));
            // Without an address, the Google account's own entry names it for its brand accounts.
            if google.is_none() && page_id.is_none() {
                google = gaia.map(str::to_owned);
            }
            let name = text(&item["accountName"]);
            if item["isDisabled"].as_bool() == Some(true) || name.is_empty() {
                continue;
            }
            let handle = Some(text(&item["channelHandle"])).filter(|handle| !handle.is_empty());
            accounts.push(Account {
                identity: Identity { index, page_id },
                name,
                email: email.clone(),
                handle,
                google: String::new(),
            });
        }
        let google = google.unwrap_or_else(|| format!("account {index}"));
        for account in accounts.iter_mut().filter(|account| account.google.is_empty()) {
            account.google = google.clone();
        }
    }
    accounts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_sign_in_came_from_survives_its_file() {
        let origin = Origin { browser: Some("brave+gnomekeyring".into()), user: Some("UCabc".into()) };
        let text = format!("# Netscape HTTP Cookie File\n{}{}", Identity::default().notes(), origin.notes());
        assert_eq!(Origin::parse(&text), origin);
        assert_eq!(Identity::parse(&text), Identity::default(), "next to the account's own notes");
        // A file from before 0.5.3, and a pasted header's.
        assert_eq!(Origin::parse("# cloofy-authuser: 1\n"), Origin::default());
        assert_eq!(Origin::default().notes(), "");
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/accounts_list.json")).unwrap()
    }

    /// An account item as YouTube sends it, with a brand account's page ID where given.
    fn item(name: &str, handle: &str, gaia: &str, page_id: Option<&str>, disabled: bool) -> Value {
        let mut tokens = vec![json!({ "accountStateToken": { "hasChannel": true, "obfuscatedGaiaId": gaia } })];
        if let Some(page_id) = page_id {
            tokens.push(json!({ "pageIdToken": { "pageId": page_id } }));
        }
        json!({ "accountItem": {
            "accountName": { "runs": [{ "text": name }] },
            "channelHandle": { "runs": [{ "text": handle }] },
            "isDisabled": disabled,
            "serviceEndpoint": { "selectActiveIdentityEndpoint": { "supportedTokens": tokens } }
        } })
    }

    #[test]
    fn reads_the_google_account_with_its_address() {
        let accounts = parse(&fixture(), 1);
        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert_eq!(account.identity, Identity { index: 1, page_id: None });
        assert_eq!(account.name, "Ada Example");
        assert_eq!(account.email.as_deref(), Some("ada@example.com"));
        assert_eq!(account.handle.as_deref(), Some("@adaexample"));
        assert_eq!(account.label(), "Ada Example · ada@example.com");
    }

    #[test]
    fn reads_brand_accounts_and_leaves_out_disabled_ones() {
        let page = json!({ "actions": [{ "accountSectionListRenderer": {
            "header": { "googleAccountHeaderRenderer": { "email": { "runs": [{ "text": "ada@example.com" }] } } },
            "contents": [{ "accountItemSectionRenderer": { "contents": [
                item("Ada Example", "@ada", "100000000000000000001", None, false),
                item("Ada's Band", "@adasband", "100000000000000000002", Some("100000000000000000099"), false),
                item("Old Channel", "", "100000000000000000003", Some("100000000000000000098"), true),
            ] } }]
        } }] });
        let accounts = parse(&page, 2);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].identity, Identity { index: 2, page_id: Some("100000000000000000099".into()) });
        assert_eq!(accounts[1].label(), "Ada's Band · brand account of ada@example.com");
        assert!(accounts[0].same_as(&parse(&page, 0)[0]), "the same account at another index");
        assert!(!accounts[0].same_as(&accounts[1]));
    }

    #[test]
    fn without_an_address_the_google_account_tells_accounts_apart() {
        let page = |gaia: &str| {
            json!({ "accountSectionListRenderer": { "contents": [{ "accountItemSectionRenderer": { "contents": [
                item("Ada Example", "@ada", gaia, None, false),
                item("Ada's Band", "", "100000000000000000002", Some("100000000000000000099"), false),
            ] } }] } })
        };
        let first = parse(&page("100000000000000000001"), 0);
        let other = parse(&page("100000000000000000005"), 1);
        assert_eq!(first[0].label(), "Ada Example · @ada");
        assert_eq!(first[1].label(), "Ada's Band");
        assert!(first[1].same_as(&parse(&page("100000000000000000001"), 1)[1]));
        assert!(!first[0].same_as(&other[0]));
        assert!(!first[1].same_as(&other[1]), "brand accounts of another Google account");
    }

    #[test]
    fn a_signed_out_page_has_no_accounts() {
        assert!(parse(&json!({ "actions": [{ "getMultiPageMenuAction": { "menu": {} } }] }), 5).is_empty());
    }

    #[test]
    fn the_account_is_kept_in_the_cookies_file() {
        let brand = Identity { index: 2, page_id: Some("100000000000000000099".into()) };
        assert_eq!(Identity::parse(&format!("# Netscape HTTP Cookie File\n{}", brand.notes())), brand);
        let first = Identity::default();
        assert_eq!(first.notes(), "# cloofy-authuser: 0\n");
        assert_eq!(Identity::parse(&first.notes()), first);
        // Sign-ins saved before cloofy kept the account act as the first one.
        assert_eq!(Identity::parse("# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tx\n"), first);
    }
}

//! Signed-in account's display name and avatar, from YouTube Music's own account menu
//! request (rustypipe doesn't expose it). Authenticated the way the website does it:
//! session cookies + `SAPISIDHASH` (SHA-1 of "<time> <SAPISID> <origin>").
use anyhow::{Result, anyhow};
use sha1::{Digest, Sha1};

const ORIGIN: &str = "https://music.youtube.com";

pub struct Account {
    pub name: String,
    pub avatar: Option<String>,
}

fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
}

fn sapisid_hash_for(sapisid: &str, now: u64, origin: &str) -> String {
    let digest = Sha1::digest(format!("{now} {sapisid} {origin}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {now}_{hex}")
}

fn sapisid_hash(sapisid: &str, now: u64) -> String {
    sapisid_hash_for(sapisid, now, ORIGIN)
}

/// `Authorization` header value for a signed-in request from `origin`.
pub fn authorization(cookie_header: &str, origin: &str) -> Option<String> {
    let sapisid = cookie_value(cookie_header, "SAPISID").or_else(|| cookie_value(cookie_header, "__Secure-3PAPISID"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(sapisid_hash_for(sapisid, now, origin))
}

/// Depth-first search for the first object under `key` anywhere in the JSON tree.
fn find<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|x| find(x, key))),
        serde_json::Value::Array(a) => a.iter().find_map(|x| find(x, key)),
        _ => None,
    }
}

/// A signed-in YouTube Music API call (`endpoint` e.g. "account/account_menu"), as the website makes it.
async fn post(http: &reqwest::Client, cookie_header: &str, endpoint: &str, extra: serde_json::Value) -> Result<serde_json::Value> {
    let sapisid = cookie_value(cookie_header, "SAPISID")
        .or_else(|| cookie_value(cookie_header, "__Secure-3PAPISID"))
        .ok_or_else(|| anyhow!("no SAPISID in session"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let mut body = serde_json::json!({ "context": { "client": {
        "clientName": "WEB_REMIX", "clientVersion": "1.20250101.01.00", "hl": "en" } } });
    if let (Some(b), Some(x)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(x.clone());
    }
    Ok(http
        .post(format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
        .header("Cookie", cookie_header)
        .header("Authorization", sapisid_hash(sapisid, now))
        .header("Origin", ORIGIN)
        .header("X-Origin", ORIGIN)
        .header("X-Goog-AuthUser", "0")
        .header("X-YouTube-Client-Name", "67")
        .header("X-YouTube-Client-Version", "1.20250101.01.00")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Like or unlike a song on the account, like the heart on YouTube Music does.
/// rustypipe can read likes but not change them.
pub async fn set_like(http: &reqwest::Client, cookie_header: &str, video_id: &str, liked: bool) -> Result<()> {
    let endpoint = if liked { "like/like" } else { "like/removelike" };
    post(http, cookie_header, endpoint, serde_json::json!({ "target": { "videoId": video_id } })).await?;
    Ok(())
}

// ------------------------------------------------------------ playlists on the account
// The same calls YouTube Music's website makes to create and edit playlists (rustypipe only
// reads them). Used to publish a local playlist for sharing (share.rs).

/// Songs per request when filling a playlist; YouTube rejects very large batches.
const BATCH: usize = 100;

/// `browse/edit_playlist` answers `"status": "STATUS_SUCCEEDED"` when every action applied.
fn check_edit(resp: &serde_json::Value) -> Result<()> {
    match resp["status"].as_str() {
        Some(s) if s.contains("SUCCEEDED") => Ok(()),
        s => Err(anyhow!("YouTube refused the playlist change ({})", s.unwrap_or("no status"))),
    }
}

/// Create an unlisted playlist (anyone with the link can open it, it isn't listed or
/// searchable) holding `video_ids` in order. Returns its id.
pub async fn create_playlist(http: &reqwest::Client, cookie_header: &str, title: &str, video_ids: &[String]) -> Result<String> {
    let (first, rest) = video_ids.split_at(video_ids.len().min(BATCH));
    let resp = post(http, cookie_header, "playlist/create", serde_json::json!({
        "title": title, "description": "", "privacyStatus": "UNLISTED", "videoIds": first,
    }))
    .await?;
    let id = resp["playlistId"].as_str().ok_or_else(|| anyhow!("YouTube didn't create the playlist"))?.to_string();
    if let Err(err) = add_to_playlist(http, cookie_header, &id, rest).await {
        // don't leave a half-filled copy behind
        let _ = delete_playlist(http, cookie_header, &id).await;
        return Err(err);
    }
    Ok(id)
}

/// Append songs to the end of a playlist on the account.
pub async fn add_to_playlist(http: &reqwest::Client, cookie_header: &str, playlist_id: &str, video_ids: &[String]) -> Result<()> {
    for chunk in video_ids.chunks(BATCH) {
        let actions: Vec<_> = chunk
            .iter()
            .map(|v| serde_json::json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": v, "dedupeOption": "DEDUPE_OPTION_SKIP" }))
            .collect();
        let resp = post(http, cookie_header, "browse/edit_playlist", serde_json::json!({ "playlistId": playlist_id, "actions": actions })).await?;
        check_edit(&resp)?;
    }
    Ok(())
}

pub async fn rename_playlist(http: &reqwest::Client, cookie_header: &str, playlist_id: &str, title: &str) -> Result<()> {
    let actions = serde_json::json!([{ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": title }]);
    let resp = post(http, cookie_header, "browse/edit_playlist", serde_json::json!({ "playlistId": playlist_id, "actions": actions })).await?;
    check_edit(&resp)
}

pub async fn delete_playlist(http: &reqwest::Client, cookie_header: &str, playlist_id: &str) -> Result<()> {
    post(http, cookie_header, "playlist/delete", serde_json::json!({ "playlistId": playlist_id })).await?;
    Ok(())
}

pub async fn fetch(http: &reqwest::Client, cookie_header: &str) -> Result<Account> {
    let resp = post(http, cookie_header, "account/account_menu", serde_json::json!({})).await?;
    let header = find(&resp, "activeAccountHeaderRenderer").ok_or_else(|| anyhow!("no account header in response"))?;
    let name = header["accountName"]["runs"][0]["text"]
        .as_str()
        .or_else(|| header["accountName"]["simpleText"].as_str())
        .ok_or_else(|| anyhow!("no account name"))?
        .to_string();
    let avatar = header["accountPhoto"]["thumbnails"]
        .as_array()
        .and_then(|t| t.last())
        .and_then(|t| t["url"].as_str())
        .map(str::to_string);
    Ok(Account { name, avatar })
}

/// Find the cookie header inside rustypipe's (decrypted) cache JSON.
pub fn cookie_from_cache(cache_json: &str) -> Option<String> {
    fn walk(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::String(s) if s.contains("SAPISID=") => Some(s.clone()),
            serde_json::Value::Object(m) => m.values().find_map(walk),
            serde_json::Value::Array(a) => a.iter().find_map(walk),
            _ => None,
        }
    }
    walk(&serde_json::from_str(cache_json).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_and_hash() {
        let h = "A=1; SAPISID=abc/def; B=2";
        assert_eq!(cookie_value(h, "SAPISID"), Some("abc/def"));
        assert_eq!(cookie_value(h, "SAPISI"), None);
        // known vector: sha1("1 abc https://music.youtube.com")
        let s = sapisid_hash("abc", 1);
        assert!(s.starts_with("SAPISIDHASH 1_") && s.len() == "SAPISIDHASH 1_".len() + 40);
        assert_eq!(cookie_from_cache(r#"{"x":{"cookie":"SID=1; SAPISID=z"}}"#).as_deref(), Some("SID=1; SAPISID=z"));
    }

    #[test]
    fn edit_status() {
        assert!(check_edit(&serde_json::json!({ "status": "STATUS_SUCCEEDED" })).is_ok());
        assert!(check_edit(&serde_json::json!({ "status": "STATUS_FAILED" })).is_err());
        assert!(check_edit(&serde_json::json!({})).is_err());
    }
}

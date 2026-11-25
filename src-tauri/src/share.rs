//! Sharing playlists between people through YouTube: a local playlist is published to the
//! signed-in account as an unlisted playlist, and its music.youtube.com link is what gets
//! shared. Anyone can open that link (in this app, YouTube Music or a browser) and save the
//! playlist to their own library. The app has no server of its own; YouTube is the carrier.

/// What publishing a local playlist was, last time: the YouTube copy and what it held.
#[derive(Debug, Clone, PartialEq)]
pub struct Published {
    pub yt_id: String,
    pub name: String,
    /// Track ids in the order they were published.
    pub ids: Vec<String>,
}

/// What to do on YouTube to make the shared copy match the local playlist.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Nothing changed: hand out the same link again.
    Reuse,
    /// Songs were only added at the end and/or the playlist was renamed: update the same
    /// YouTube playlist, so links already sent keep working and show the new songs.
    Update { add: Vec<String>, rename: bool },
    /// Never published, or songs were removed or reordered: publish a new copy
    /// (and delete the old one).
    Create,
}

pub fn plan(prev: Option<&Published>, name: &str, ids: &[String]) -> Plan {
    let Some(p) = prev else { return Plan::Create };
    if !ids.starts_with(&p.ids) {
        return Plan::Create;
    }
    let add = ids[p.ids.len()..].to_vec();
    let rename = p.name != name;
    if add.is_empty() && !rename { Plan::Reuse } else { Plan::Update { add, rename } }
}

/// YouTube playlists hold at most 5000 songs.
pub const MAX_TRACKS: usize = 5000;

pub fn share_url(yt_id: &str) -> String {
    format!("https://music.youtube.com/playlist?list={yt_id}")
}

fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Prefixes of playlists people share: user playlists (PL), albums (OLAK5uy_) and
/// YouTube Music's own playlists (RDCLAK5uy_). Short system ids (LM, WL, LL...) are
/// private to whoever opens them, so they're not accepted.
fn looks_like_playlist_id(id: &str) -> bool {
    let known = (id.starts_with("PL") && id.len() >= 18)
        || id.starts_with("OLAK5uy_")
        || id.starts_with("RDCLAK5uy_");
    known && id.len() <= 64 && id.chars().all(id_char)
}

/// The playlist id in a YouTube / YouTube Music link (any `...?list=<id>` or `&list=<id>`,
/// also inside a longer message), or a bare playlist id. `None` when there isn't one.
pub fn playlist_id_from_link(text: &str) -> Option<String> {
    let text = text.trim();
    let mut rest = text;
    while let Some(at) = rest.find("list=") {
        let before = rest[..at].chars().last();
        let after = &rest[at + "list=".len()..];
        if matches!(before, Some('?' | '&')) {
            let id: String = after.chars().take_while(|&c| id_char(c)).collect();
            let id = id.strip_prefix("VL").filter(|s| looks_like_playlist_id(s)).unwrap_or(&id);
            if looks_like_playlist_id(id) {
                return Some(id.to_string());
            }
        }
        rest = after;
    }
    // A bare id (as YouTube shows it, or the browse form "VL<id>").
    let id = text.strip_prefix("VL").filter(|s| looks_like_playlist_id(s)).unwrap_or(text);
    looks_like_playlist_id(id).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PL: &str = "PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf";

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_playlist_in_links() {
        for link in [
            format!("https://music.youtube.com/playlist?list={PL}"),
            format!("https://www.youtube.com/playlist?list={PL}&si=abc123"),
            format!("https://youtube.com/watch?v=dQw4w9WgXcQ&list={PL}&index=3"),
            format!("https://youtu.be/dQw4w9WgXcQ?list={PL}"),
            format!("https://m.youtube.com/playlist?list={PL}"),
            format!("Послухай: https://music.youtube.com/playlist?list={PL} 🔥"),
            format!("  {PL}  "),
            format!("VL{PL}"),
            format!("https://music.youtube.com/browse/VL{PL}?list=VL{PL}"),
        ] {
            assert_eq!(playlist_id_from_link(&link).as_deref(), Some(PL), "{link}");
        }
        let album = "OLAK5uy_kRUVXUgQh1Kt5tF6Zt9dXvYqVq5mPZ3gE";
        assert_eq!(playlist_id_from_link(&format!("https://music.youtube.com/playlist?list={album}")).as_deref(), Some(album));
        let ytm = "RDCLAK5uy_kmPRjHDECIcuVwnKsx2Ng7fyNgFKWNJFs";
        assert_eq!(playlist_id_from_link(ytm).as_deref(), Some(ytm));
    }

    #[test]
    fn rejects_non_playlists() {
        for s in [
            "",
            "океан ельзи",
            "daft punk",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/playlist?list=LM",
            "https://www.youtube.com/playlist?list=WL",
            "PLAYLIST",
            "https://example.com/?blacklist=PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf",
        ] {
            assert_eq!(playlist_id_from_link(s), None, "{s}");
        }
    }

    #[test]
    fn plans_publishing() {
        let prev = Published { yt_id: "PLx".into(), name: "Mix".into(), ids: ids(&["a", "b"]) };
        assert_eq!(plan(None, "Mix", &ids(&["a"])), Plan::Create);
        assert_eq!(plan(Some(&prev), "Mix", &ids(&["a", "b"])), Plan::Reuse);
        assert_eq!(plan(Some(&prev), "Mix", &ids(&["a", "b", "c"])), Plan::Update { add: ids(&["c"]), rename: false });
        assert_eq!(plan(Some(&prev), "New", &ids(&["a", "b"])), Plan::Update { add: vec![], rename: true });
        assert_eq!(plan(Some(&prev), "Mix", &ids(&["b", "a"])), Plan::Create);
        assert_eq!(plan(Some(&prev), "Mix", &ids(&["a"])), Plan::Create);
        assert_eq!(share_url("PLx"), "https://music.youtube.com/playlist?list=PLx");
    }
}

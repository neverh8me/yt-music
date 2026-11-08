//! "Made for you" home sections, built from local listening history and likes
//! (nothing about your taste leaves the device except the YouTube lookups for seeds).
//!
//! - your_mix:     radio blended from your most-played and some liked tracks
//! - more_like:    artists similar to your top artist
//! - your_artists: albums from the artists you play most
//! - rediscover:   liked tracks you haven't played for a month
//!
//! Cached for a few hours so Home opens instantly.
use crate::db::now;
use crate::models::{AlbumCard, HomeSection, Track};
use crate::state::Core;
use std::collections::{HashMap, HashSet};

const CACHE_KEY: &str = "home_personal";
const TTL_MS: i64 = 3 * 3600 * 1000;
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(serde::Serialize, serde::Deserialize)]
struct Cached {
    ts: i64,
    sections: Vec<HomeSection>,
}

/// Rank artists by how much you play/like them. Only artists with a YouTube id count.
pub fn top_artists(played: &[(Track, u32)], liked: &[Track], n: usize) -> Vec<(String, String)> {
    let mut score: HashMap<String, (String, u32)> = HashMap::new();
    let mut bump = |t: &Track, w: u32| {
        for a in &t.artists {
            if let Some(id) = &a.id {
                score.entry(id.clone()).or_insert((a.name.clone(), 0)).1 += w;
            }
        }
    };
    for (t, plays) in played {
        bump(t, plays * 2);
    }
    for t in liked {
        bump(t, 3);
    }
    let mut v: Vec<_> = score.into_iter().map(|(id, (name, s))| (s, id, name)).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
    v.into_iter().take(n).map(|(_, id, name)| (id, name)).collect()
}

/// Round-robin merge of several radios, skipping duplicates.
pub fn interleave(lists: Vec<Vec<Track>>, max: usize) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for l in &lists {
            if let Some(t) = l.get(i)
                && seen.insert(t.id.clone())
            {
                out.push(t.clone());
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

pub async fn personal(core: &Core) -> Vec<HomeSection> {
    if let Some(c) = core.db.cache_get(CACHE_KEY).and_then(|s| serde_json::from_str::<Cached>(&s).ok())
        && now() - c.ts < TTL_MS
    {
        return c.sections;
    }
    let sections = build(core).await;
    if let Ok(json) = serde_json::to_string(&Cached { ts: now(), sections: sections.clone() }) {
        let _ = core.db.cache_put(CACHE_KEY, &json);
    }
    sections
}

async fn build(core: &Core) -> Vec<HomeSection> {
    let played = core.db.top_played(now() - 60 * DAY_MS, 40).unwrap_or_default();
    let liked = core.db.liked_sample(8).unwrap_or_default();
    if played.is_empty() && liked.is_empty() {
        return vec![]; // brand-new user: Home shows regional charts only
    }

    // Seeds: your 3 most played + 2 random liked tracks.
    let mut seeds: Vec<Track> = played.iter().take(3).map(|(t, _)| t.clone()).collect();
    for t in liked.iter().take(2) {
        if !seeds.iter().any(|s| s.id == t.id) {
            seeds.push(t.clone());
        }
    }
    let artists = top_artists(&played, &liked, 3);

    let radios = futures_util::future::join_all(seeds.iter().map(|s| core.yt.radio(&s.id)));
    let pages = futures_util::future::join_all(artists.iter().map(|(id, _)| core.yt.artist(id)));
    let (radios, pages) = tokio::join!(radios, pages);

    let mut out = Vec::new();

    let mix = interleave(radios.into_iter().filter_map(Result::ok).collect(), 30);
    if !mix.is_empty() {
        out.push(HomeSection { title: "your_mix".into(), tracks: mix, ..Default::default() });
    }

    let pages: Vec<_> = pages.into_iter().filter_map(Result::ok).collect();
    if let Some(first) = pages.first()
        && !first.similar.is_empty()
    {
        out.push(HomeSection {
            title: "more_like".into(),
            param: Some(first.name.clone()),
            artists: first.similar.iter().take(12).cloned().collect(),
            ..Default::default()
        });
    }

    // A few releases from each favourite artist, interleaved so one artist doesn't dominate.
    let per_artist: Vec<Vec<AlbumCard>> = pages
        .iter()
        .map(|p| p.albums.iter().chain(p.singles.iter()).take(4).cloned().collect())
        .collect();
    let mut albums = Vec::new();
    for i in 0..4 {
        for list in &per_artist {
            if let Some(a) = list.get(i) {
                albums.push(a.clone());
            }
        }
    }
    if !albums.is_empty() {
        out.push(HomeSection { title: "your_artists".into(), albums, ..Default::default() });
    }

    let rediscover = core.db.liked_not_played_since(now() - 30 * DAY_MS, 10).unwrap_or_default();
    if rediscover.len() >= 3 {
        out.push(HomeSection { title: "rediscover".into(), tracks: rediscover, ..Default::default() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ArtistRef;

    fn t(id: &str, artist: (&str, &str)) -> Track {
        Track {
            id: id.into(),
            title: id.into(),
            artists: vec![ArtistRef { id: Some(artist.0.into()), name: artist.1.into() }],
            album: None,
            duration: None,
            cover: None,
            plays: None,
        }
    }

    #[test]
    fn ranks_artists_by_plays_and_likes() {
        let played = vec![(t("a", ("q", "Queen")), 10), (t("b", ("m", "Muse")), 2)];
        let liked = vec![t("c", ("m", "Muse")), t("d", ("r", "Radiohead"))];
        let top = top_artists(&played, &liked, 3);
        assert_eq!(top[0].1, "Queen"); // 20
        assert_eq!(top[1].1, "Muse"); // 4 + 3
        assert_eq!(top[2].1, "Radiohead"); // 3
    }

    #[test]
    fn interleaves_without_duplicates() {
        let a = vec![t("1", ("x", "X")), t("2", ("x", "X"))];
        let b = vec![t("1", ("x", "X")), t("3", ("x", "X"))];
        let ids: Vec<_> = interleave(vec![a, b], 10).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["1", "2", "3"]);
    }
}

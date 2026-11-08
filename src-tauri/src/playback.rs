//! Tauri commands around the Rust queue engine (`player::Queue`).
//! After every change the queue is persisted and a `player-state` event is emitted;
//! when a new track starts we record history, pre-resolve the next stream and top up
//! the queue with radio tracks when it is about to run out.
use crate::models::Track;
use crate::player::{Context, Queue, Snapshot, Step};
use crate::state::Core;
use crate::yt::Codec;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type Res<T> = Result<T, String>;
type St<'a> = State<'a, Arc<Core>>;

#[derive(Serialize)]
pub struct StepReply {
    pub step: Step,
    pub state: Snapshot,
}

/// Persist + broadcast; kick off side effects when playback (re)started.
fn publish(app: &AppHandle, core: &Arc<Core>, q: &Queue, started: bool) -> Snapshot {
    let snap = q.snapshot();
    if let Ok(json) = serde_json::to_string(&q.for_storage()) {
        let _ = core.db.set_setting("queue", &json);
    }
    let _ = app.emit("player-state", &snap);
    if started {
        on_started(app, core, q);
    }
    snap
}

fn on_started(app: &AppHandle, core: &Arc<Core>, q: &Queue) {
    let Some(cur) = q.current() else { return };
    let _ = core.db.record_play(&cur.track);
    crate::integrations::notify_track(app, core, &cur.track);
    // Pre-resolve the next two stream URLs so skipping is instant.
    for next in q.index.map(|i| q.items.iter().skip(i + 1).take(2)).into_iter().flatten() {
        let (core, id) = (core.clone(), next.track.id.clone());
        tauri::async_runtime::spawn(async move {
            if core.db.download_path(&id).is_none() {
                let _ = core.yt.stream(&id, Codec::Aac).await;
            }
        });
    }
    // Close to the end with autoplay on: top up with radio in the background.
    let near_end = q.index.is_some_and(|i| i + 2 >= q.items.len());
    if q.autoplay && q.repeat == crate::player::Repeat::Off && near_end {
        let (app, core, seed) = (app.clone(), core.clone(), cur.track.id.clone());
        tauri::async_runtime::spawn(async move {
            if let Ok(tracks) = core.yt.radio(&seed).await {
                let mut q = core.queue.lock().await;
                if q.extend_radio(tracks) > 0 {
                    publish(&app, &core, &q, false);
                }
            }
        });
    }
}

#[tauri::command]
pub async fn player_state(core: St<'_>) -> Res<Snapshot> {
    Ok(core.queue.lock().await.snapshot())
}

#[tauri::command]
pub async fn player_play_list(app: AppHandle, core: St<'_>, tracks: Vec<Track>, start: usize, context: Option<Context>) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.play_list(tracks, start, context);
    Ok(publish(&app, &core, &q, true))
}

#[tauri::command]
pub async fn player_play_next(app: AppHandle, core: St<'_>, tracks: Vec<Track>) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    let was_empty = q.index.is_none();
    q.play_next(tracks);
    Ok(publish(&app, &core, &q, was_empty))
}

#[tauri::command]
pub async fn player_add_queue(app: AppHandle, core: St<'_>, tracks: Vec<Track>) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    let was_empty = q.index.is_none();
    q.add_to_queue(tracks);
    Ok(publish(&app, &core, &q, was_empty))
}

#[tauri::command]
pub async fn player_remove(app: AppHandle, core: St<'_>, index: usize) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.remove(index);
    Ok(publish(&app, &core, &q, false))
}

#[tauri::command]
pub async fn player_jump(app: AppHandle, core: St<'_>, index: usize) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.jump(index);
    Ok(publish(&app, &core, &q, true))
}

/// `auto` = the current track ended by itself (repeat-one applies).
#[tauri::command]
pub async fn player_next(app: AppHandle, core: St<'_>, auto: bool) -> Res<StepReply> {
    let core = core.inner().clone();
    // "Stop after this track" sleep timer.
    if auto && fire_end_of_track(&app, &core) {
        let q = core.queue.lock().await;
        return Ok(StepReply { step: Step::Stop, state: q.snapshot() });
    }
    let mut q = core.queue.lock().await;
    if let Some(step) = q.next(auto) {
        let state = publish(&app, &core, &q, step == Step::Play);
        return Ok(StepReply { step, state });
    }
    // End of queue: extend with radio (autoplay) or stop.
    let seed = q.current().map(|c| c.track.id.clone());
    if !(q.autoplay && seed.is_some()) {
        return Ok(StepReply { step: Step::Stop, state: q.snapshot() });
    }
    drop(q);
    let tracks = core.yt.radio(&seed.unwrap_or_default()).await.unwrap_or_default();
    let mut q = core.queue.lock().await;
    if q.extend_radio(tracks) > 0
        && let Some(step) = q.next(false)
    {
        let state = publish(&app, &core, &q, true);
        return Ok(StepReply { step, state });
    }
    Ok(StepReply { step: Step::Stop, state: q.snapshot() })
}

#[tauri::command]
pub async fn player_prev(app: AppHandle, core: St<'_>, position: f64) -> Res<StepReply> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    let step = q.prev(position);
    let state = publish(&app, &core, &q, step == Step::Play);
    Ok(StepReply { step, state })
}

#[tauri::command]
pub async fn player_toggle_shuffle(app: AppHandle, core: St<'_>) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.toggle_shuffle();
    Ok(publish(&app, &core, &q, false))
}

#[tauri::command]
pub async fn player_cycle_repeat(app: AppHandle, core: St<'_>) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.cycle_repeat();
    Ok(publish(&app, &core, &q, false))
}

#[tauri::command]
pub async fn player_set_autoplay(app: AppHandle, core: St<'_>, on: bool) -> Res<Snapshot> {
    let core = core.inner().clone();
    let mut q = core.queue.lock().await;
    q.autoplay = on;
    Ok(publish(&app, &core, &q, false))
}

// ------------------------------------------------------------------ sleep timer
#[derive(Serialize)]
pub struct SleepState {
    pub remaining_s: Option<u64>,
    pub end_of_track: bool,
}

fn sleep_state(core: &Core) -> SleepState {
    let s = core.sleep.lock().unwrap();
    SleepState {
        remaining_s: s.deadline_ms.map(|d| ((d - crate::db::now()).max(0) / 1000) as u64),
        end_of_track: s.end_of_track,
    }
}

fn fire_end_of_track(app: &AppHandle, core: &Core) -> bool {
    let mut s = core.sleep.lock().unwrap();
    if !s.end_of_track {
        return false;
    }
    *s = crate::state::SleepTimer { generation: s.generation + 1, ..Default::default() };
    let _ = app.emit("sleep-timer", ());
    true
}

/// `minutes`: stop after N minutes; `end_of_track`: stop when the current song ends;
/// neither: cancel.
#[tauri::command]
pub fn set_sleep_timer(app: AppHandle, core: St, minutes: Option<u32>, end_of_track: bool) -> Res<SleepState> {
    let core = core.inner().clone();
    let generation = {
        let mut s = core.sleep.lock().unwrap();
        s.generation += 1;
        s.end_of_track = end_of_track;
        s.deadline_ms = minutes.filter(|m| *m > 0).map(|m| crate::db::now() + m as i64 * 60_000);
        s.generation
    };
    if let Some(m) = minutes.filter(|m| *m > 0) {
        let core2 = core.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(m as u64 * 60)).await;
            let fire = {
                let mut s = core2.sleep.lock().unwrap();
                let current = s.generation == generation;
                if current {
                    *s = crate::state::SleepTimer { generation: s.generation + 1, ..Default::default() };
                }
                current
            };
            if fire {
                let _ = app.emit("sleep-timer", ());
            }
        });
    }
    Ok(sleep_state(&core))
}

#[tauri::command]
pub fn sleep_timer_state(core: St) -> Res<SleepState> {
    Ok(sleep_state(&core))
}

// ------------------------------------------------------------------ Discord presence
/// The UI reports play/pause/seek; Rust knows the track and talks to Discord.
#[tauri::command]
pub async fn presence(core: St<'_>, playing: bool, position: f64) -> Res<()> {
    use crate::integrations::Presence;
    if !crate::integrations::enabled(&core, "discord_presence") {
        core.discord.update(Presence::Off);
        return Ok(());
    }
    let track = core.queue.lock().await.current().map(|c| c.track.clone());
    core.discord.update(match track {
        Some(track) if playing => Presence::Playing { track, position_s: position },
        Some(_) => Presence::Paused,
        None => Presence::Off,
    });
    Ok(())
}

/// Simple on/off switches stored in settings: notifications, Discord, global hotkeys.
#[tauri::command]
pub fn set_toggle(core: St, key: String, on: bool) -> Res<()> {
    const KEYS: [&str; 3] = ["notify_tracks", "discord_presence", "global_hotkeys"];
    if !KEYS.contains(&key.as_str()) {
        return Err("unknown setting".into());
    }
    core.db.set_setting(&key, if on { "1" } else { "0" }).map_err(|e| e.to_string())
}

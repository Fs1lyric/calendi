//! lyric-cal-booker: serves the public booking page and its JSON API from
//! the same SQLite file the desktop app uses.

mod remind;

use axum::extract::{Path, Query, State as Ext};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use lyric_cal_core::db::{BookRequest, Db, Error, EventType};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// The booking page: one static file, everything else comes from the API.
const PAGE: &str = include_str!("page.html");

/// Shared by every request; the clock is a field so tests can fix it.
struct State {
    db: Mutex<Db>,
    tz: Tz,
    now: fn() -> DateTime<Utc>,
}

type Shared = Ext<Arc<State>>;

fn app(state: Arc<State>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/types/{slug}", get(event_type))
        .route("/api/slots", get(slots))
        .route("/api/book", post(book))
        .route("/{slug}", get(page))
        .with_state(state)
}

struct Failure(StatusCode, String);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        let status = match e {
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Taken => StatusCode::CONFLICT,
            Error::Sqlite(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message =
            if status == StatusCode::INTERNAL_SERVER_ERROR { "something broke".into() } else { e.to_string() };
        Failure(status, message)
    }
}

fn bad(msg: impl Into<String>) -> Failure {
    Failure(StatusCode::BAD_REQUEST, msg.into())
}

/// An active event type, or 404.
fn open_type(db: &Db, slug: &str) -> Result<EventType, Failure> {
    match db.event_type_by_slug(slug)? {
        Some(t) if t.active => Ok(t),
        _ => Err(Failure(StatusCode::NOT_FOUND, format!("no booking link /{slug}"))),
    }
}

async fn page(s: Shared, Path(slug): Path<String>) -> Response {
    let found = open_type(&s.db.lock().unwrap(), &slug).is_ok();
    if found {
        Html(PAGE).into_response()
    } else {
        (StatusCode::NOT_FOUND, Html(PAGE)).into_response()
    }
}

/// What the booking page shows about a type; nothing about the organizer's rules.
#[derive(Serialize)]
struct PublicType {
    slug: String,
    title: String,
    description: String,
    duration_min: i64,
    color: Option<String>,
    max_days_ahead: i64,
}

async fn event_type(s: Shared, Path(slug): Path<String>) -> Result<Json<PublicType>, Failure> {
    let t = open_type(&s.db.lock().unwrap(), &slug)?.input;
    Ok(Json(PublicType {
        slug: t.slug,
        title: t.title,
        description: t.description,
        duration_min: t.duration_min,
        color: t.color,
        max_days_ahead: t.max_days_ahead,
    }))
}

#[derive(Deserialize)]
struct SlotQuery {
    #[serde(rename = "type")]
    slug: String,
    from: String,
    to: String,
    tz: String,
}

async fn slots(s: Shared, Query(q): Query<SlotQuery>) -> Result<Json<serde_json::Value>, Failure> {
    let instant = |v: &str| {
        DateTime::parse_from_rfc3339(v).map(|t| t.with_timezone(&Utc)).map_err(|_| bad(format!("not a time: {v}")))
    };
    let (from, to) = (instant(&q.from)?, instant(&q.to)?);
    if to <= from || to - from > Duration::days(62) {
        return Err(bad("ask for up to 62 days at once"));
    }
    let tz: Tz = q.tz.parse().map_err(|_| bad(format!("unknown time zone {}", q.tz)))?;
    let db = s.db.lock().unwrap();
    let t = open_type(&db, &q.slug)?;
    let list = db.slots_for(&t, s.tz, tz, (s.now)(), from, to)?;
    let list: Vec<String> = list.iter().map(|t| t.with_timezone(&tz).to_rfc3339()).collect();
    Ok(Json(json!({ "slots": list })))
}

async fn book(s: Shared, Json(req): Json<BookRequest>) -> Result<impl IntoResponse, Failure> {
    let b = s.db.lock().unwrap().book(&req, s.tz, (s.now)())?;
    let body = json!({ "title": b.type_title, "start": b.start, "end": b.end, "guest_name": b.guest_name });
    Ok((StatusCode::CREATED, Json(body)))
}

const USAGE: &str = "usage: lyric-cal-booker [--port 8787] [--bind 127.0.0.1]";

/// The organizer's zone: the `tz` setting, else the system's.
fn organizer_tz(db: &Db) -> Tz {
    let setting = db.setting("tz").ok().flatten();
    setting.or_else(|| iana_time_zone::get_timezone().ok()).and_then(|z| z.parse().ok()).unwrap_or(Tz::UTC)
}

fn main() {
    let (mut port, mut bind) = (8787u16, "127.0.0.1".to_string());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--port", Some(p)) if p.parse::<u16>().is_ok() => port = p.parse().unwrap(),
            ("--bind", Some(b)) => bind = b,
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let db = match Db::open(&Db::default_path()) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("lyric-cal-booker: can't open the calendar: {e}");
            std::process::exit(1);
        }
    };
    let tz = organizer_tz(&db);
    let state = Arc::new(State { db: Mutex::new(db), tz, now: Utc::now });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    runtime.block_on(async {
        let listener = match tokio::net::TcpListener::bind((bind.as_str(), port)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("lyric-cal-booker: can't listen on {bind}:{port}: {e}");
                std::process::exit(1);
            }
        };
        eprintln!("lyric-cal-booker: http://{bind}:{port} ({tz})");
        let stop = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        tokio::spawn(remind::run(state.clone()));
        axum::serve(listener, app(state)).with_graceful_shutdown(stop).await.expect("server");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use lyric_cal_core::db::{EventTypeInput, WeeklyHours};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn fixed_now() -> DateTime<Utc> {
        "2026-10-15T00:00:00Z".parse().unwrap()
    }

    /// Mondays 09:00–12:00 in Dhaka; a 30 minute "chat" and an inactive "old".
    fn router() -> Router {
        let db = Db::open_in_memory().unwrap();
        db.set_weekly_hours(&[WeeklyHours { weekday: 1, start_minute: 540, end_minute: 720 }]).unwrap();
        let mut t = EventTypeInput {
            slug: "chat".into(),
            title: "Chat".into(),
            description: "Say hi.".into(),
            duration_min: 30,
            buffer_before_min: 0,
            buffer_after_min: 0,
            min_notice_min: 0,
            max_days_ahead: 30,
            color: None,
            active: true,
        };
        db.create_event_type(&t).unwrap();
        (t.slug, t.title, t.active) = ("old".into(), "Old".into(), false);
        db.create_event_type(&t).unwrap();
        app(Arc::new(State { db: Mutex::new(db), tz: chrono_tz::Asia::Dhaka, now: fixed_now }))
    }

    async fn call(r: &Router, req: Request<Body>) -> (StatusCode, String) {
        let res = r.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    async fn get(r: &Router, uri: &str) -> (StatusCode, String) {
        call(r, Request::get(uri).body(Body::empty()).unwrap()).await
    }

    async fn post(r: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let req =
            Request::post(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let (status, text) = call(r, req).await;
        (status, serde_json::from_str(&text).unwrap())
    }

    fn booking(start: &str) -> Value {
        json!({"slug": "chat", "start": start, "guest_name": "Ana", "guest_email": "ana@example.com",
               "guest_note": "", "guest_tz": "Asia/Dhaka"})
    }

    #[tokio::test]
    async fn the_page_is_served_for_open_links_only() {
        let r = router();
        let (status, html) = get(&r, "/chat").await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<!doctype html>"));
        assert_eq!(get(&r, "/old").await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&r, "/nope").await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&r, "/health").await, (StatusCode::OK, "ok".into()));
    }

    #[tokio::test]
    async fn types_show_only_what_guests_need() {
        let r = router();
        let (status, body) = get(&r, "/api/types/chat").await;
        assert_eq!(status, StatusCode::OK);
        let t: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            t,
            json!({"slug": "chat", "title": "Chat", "description": "Say hi.", "duration_min": 30,
                              "color": null, "max_days_ahead": 30})
        );
        assert_eq!(get(&r, "/api/types/old").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn slots_come_in_the_guests_zone() {
        let r = router();
        let uri = "/api/slots?type=chat&from=2026-10-19T00:00:00Z&to=2026-10-20T00:00:00Z&tz=Europe/London";
        let (status, body) = get(&r, uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        let slots = v["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 6);
        assert_eq!(slots[0], "2026-10-19T04:00:00+01:00");

        let bad = "/api/slots?type=chat&from=2026-10-19T00:00:00Z&to=2027-10-20T00:00:00Z&tz=UTC";
        assert_eq!(get(&r, bad).await.0, StatusCode::BAD_REQUEST, "at most 62 days at once");
        let zone = "/api/slots?type=chat&from=2026-10-19T00:00:00Z&to=2026-10-20T00:00:00Z&tz=Nowhere";
        assert_eq!(get(&r, zone).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_slot_books_once() {
        let r = router();
        let (status, b) = post(&r, "/api/book", booking("2026-10-19T03:00:00Z")).await;
        assert_eq!(status, StatusCode::CREATED, "{b}");
        assert_eq!(b["start"], "2026-10-19T03:00:00Z");
        assert_eq!(b["title"], "Chat");
        assert!(b.get("id").is_none(), "no ids to guess other bookings by");

        let (status, e) = post(&r, "/api/book", booking("2026-10-19T03:00:00Z")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(e["error"], "that time is no longer free");

        let mut bad = booking("2026-10-19T03:30:00Z");
        bad["guest_email"] = json!("nope");
        assert_eq!(post(&r, "/api/book", bad).await.0, StatusCode::BAD_REQUEST);
        let mut gone = booking("2026-10-19T03:30:00Z");
        gone["slug"] = json!("old");
        assert_eq!(post(&r, "/api/book", gone).await.0, StatusCode::NOT_FOUND);
    }
}

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use maplibre_contour_rs::cache::DemCache;
use maplibre_contour_rs::config::parse_thresholds;
use maplibre_contour_rs::dem_source::{TileSource, UrlTemplate};
use maplibre_contour_rs::{ContourConfig, DemManager, Encoding, Error, TileCoord};
use serde::Deserialize;
use serde_json::{json, Value};

pub const DEFAULT_DEM_URL_PATTERN: &str =
    "https://mapeak.com/vector/data/raster-dem/{z}/{x}/{y}.webp";
pub const DEFAULT_CACHE_SIZE: usize = 64;
pub const DEFAULT_ATTRIBUTION: &str =
    "<a href='https://mapterhorn.com/attribution'>© Mapterhorn</a>";
const TILEJSON_MAX_ZOOM: u8 = 16;
const THRESHOLDS: &str = "9*500*2500~10*200*1000~11*100*500~12*50*250~13*20*100~14*10*50";
const METRIC_MULTIPLIER: f32 = 1.0;
const IMPERIAL_MULTIPLIER: f32 = 3.28084;
const DEM_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ZOOM: u8 = 30;
const CACHE_CONTROL: &str = "public, max-age=604800";
const MVT_CONTENT_TYPE: &str = "application/vnd.mapbox-vector-tile";

pub struct Settings {
    pub dem_url_pattern: String,
    pub cache_size: usize,
    pub public_url: String,
    pub attribution: String,
}

struct HttpSource {
    agent: ureq::Agent,
    template: UrlTemplate,
}

impl TileSource for HttpSource {
    fn fetch(&self, coord: TileCoord) -> maplibre_contour_rs::Result<Option<Vec<u8>>> {
        let url = self.template.resolve(coord);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Error::Source(format!("{url}: {e}")))?;
        match response.status().as_u16() {
            200 => response
                .body_mut()
                .read_to_vec()
                .map(Some)
                .map_err(|e| Error::Source(format!("{url}: {e}"))),
            404 => Ok(None),
            status => Err(Error::Source(format!("{url}: HTTP {status}"))),
        }
    }
}

struct Managers {
    metric: DemManager<HttpSource>,
    imperial: DemManager<HttpSource>,
}

#[derive(Deserialize)]
struct TileQuery {
    units: Option<String>,
}

fn contour_config(dem_url_pattern: &str, multiplier: f32) -> ContourConfig {
    ContourConfig {
        encoding: Encoding::Terrarium,
        dem_url_pattern: dem_url_pattern.to_string(),
        dem_max_zoom: 11,
        overzoom: 1,
        thresholds: parse_thresholds(THRESHOLDS),
        multiplier,
        layer_name: "contours".to_string(),
        elevation_key: "ele".to_string(),
        level_key: "level".to_string(),
        ..Default::default()
    }
}

fn manager(
    agent: &ureq::Agent,
    cache: &DemCache,
    dem_url_pattern: &str,
    multiplier: f32,
) -> DemManager<HttpSource> {
    let source = HttpSource {
        agent: agent.clone(),
        template: UrlTemplate::new(dem_url_pattern),
    };
    DemManager::new(source, contour_config(dem_url_pattern, multiplier)).with_cache(cache.clone())
}

fn tilejson(settings: &Settings, config: &ContourConfig) -> Value {
    let minzoom = config.thresholds.iter().map(|r| r.zoom).min().unwrap_or(0);
    let public_url = settings.public_url.trim_end_matches('/');
    json!({
        "tilejson": "3.0.0",
        "name": "contour",
        "attribution": settings.attribution,
        "scheme": "xyz",
        "tiles": [format!("{public_url}/{{z}}/{{x}}/{{y}}.mvt")],
        "bounds": [-180, -85.051129, 180, 85.051129],
        "minzoom": minzoom,
        "maxzoom": TILEJSON_MAX_ZOOM,
        "vector_layers": [{
            "id": config.layer_name,
            "fields": { config.elevation_key.clone(): "Number", config.level_key.clone(): "Number" },
            "minzoom": minzoom,
            "maxzoom": TILEJSON_MAX_ZOOM,
        }],
    })
}

fn parse_coord(z: u8, x: u32, y: &str) -> Option<TileCoord> {
    let y = y
        .strip_suffix(".pbf")
        .or_else(|| y.strip_suffix(".mvt"))
        .unwrap_or(y);
    let y: u32 = y.parse().ok()?;
    (z <= MAX_ZOOM && x < 1 << z && y < 1 << z).then(|| TileCoord::new(z, x, y))
}

async fn tile(
    State(managers): State<Arc<Managers>>,
    Path((z, x, y)): Path<(u8, u32, String)>,
    Query(query): Query<TileQuery>,
) -> Response {
    let Some(coord) = parse_coord(z, x, &y) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let imperial = query.units.as_deref() == Some("imperial");
    let result = tokio::task::spawn_blocking(move || {
        let manager = if imperial {
            &managers.imperial
        } else {
            &managers.metric
        };
        manager.tile(coord)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, MVT_CONTENT_TYPE),
                (header::CACHE_CONTROL, CACHE_CONTROL),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub fn router(settings: &Settings) -> Router {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(DEM_TIMEOUT))
        .build()
        .new_agent();
    let cache = DemCache::new(settings.cache_size);
    let managers = Arc::new(Managers {
        metric: manager(&agent, &cache, &settings.dem_url_pattern, METRIC_MULTIPLIER),
        imperial: manager(
            &agent,
            &cache,
            &settings.dem_url_pattern,
            IMPERIAL_MULTIPLIER,
        ),
    });
    let tilejson = Json(tilejson(
        settings,
        &contour_config(&settings.dem_url_pattern, METRIC_MULTIPLIER),
    ));
    Router::new()
        .route("/{z}/{x}/{y}", get(tile))
        .route("/tilejson.json", get(move || async move { tilejson }))
        .route("/health", get(|| async { "ok" }))
        .with_state(managers)
}

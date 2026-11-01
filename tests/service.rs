use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use dem_contour_service::{router, Settings, DEFAULT_ATTRIBUTION};
use geozero::mvt::tile::Value;
use geozero::mvt::{Message, Tile};
use http_body_util::BodyExt;
use image::{ImageFormat, Rgba, RgbaImage};
use tower::ServiceExt;

const DEM_SIZE: u32 = 256;
const MISSING_X: u32 = 100;
const BROKEN_X: u32 = 200;
const PUBLIC_URL: &str = "https://example.test/vector/data/contour";

#[derive(Clone)]
struct Dem {
    fetches: Arc<AtomicUsize>,
}

fn height(z: u32, x: u32, px: u32) -> f32 {
    let u = (x * DEM_SIZE + px) as f64 / (DEM_SIZE as f64 * (1u64 << z) as f64);
    (1000.0 + 2_000_000.0 * (u - 0.5)).max(0.0) as f32
}

fn terrarium_png(z: u32, x: u32) -> Vec<u8> {
    let mut img = RgbaImage::new(DEM_SIZE, DEM_SIZE);
    for px in 0..DEM_SIZE {
        let v = (height(z, x, px) + 32768.0) as f64;
        let r = (v / 256.0).floor();
        let g = (v - r * 256.0).floor();
        let b = ((v - r * 256.0 - g) * 256.0).floor();
        for py in 0..DEM_SIZE {
            img.put_pixel(px, py, Rgba([r as u8, g as u8, b as u8, 255]));
        }
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .unwrap();
    buf
}

async fn dem_tile(State(dem): State<Dem>, Path((z, x, _y)): Path<(u32, u32, String)>) -> Response {
    dem.fetches.fetch_add(1, Ordering::SeqCst);
    match x >> (11 - z.min(11)) {
        MISSING_X => StatusCode::NOT_FOUND.into_response(),
        BROKEN_X => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        _ => terrarium_png(z, x).into_response(),
    }
}

async fn start_dem() -> (String, Arc<AtomicUsize>) {
    let fetches = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/{z}/{x}/{y}", get(dem_tile))
        .with_state(Dem {
            fetches: fetches.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/{{z}}/{{x}}/{{y}}.png"), fetches)
}

async fn service() -> (Router, Arc<AtomicUsize>) {
    let (dem_url_pattern, fetches) = start_dem().await;
    let settings = Settings {
        dem_url_pattern,
        cache_size: 64,
        public_url: format!("{PUBLIC_URL}/"),
        attribution: DEFAULT_ATTRIBUTION.to_string(),
    };
    (router(&settings), fetches)
}

async fn get_response(app: &Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

fn contours(bytes: &[u8]) -> Vec<(f64, i64)> {
    let tile = Tile::decode(bytes).unwrap();
    assert_eq!(tile.layers.len(), 1);
    let layer = &tile.layers[0];
    assert_eq!(layer.name, "contours");
    assert_eq!(layer.extent, Some(4096));
    let value = |v: &Value| {
        v.double_value
            .or(v.float_value.map(f64::from))
            .or(v.int_value.map(|i| i as f64))
            .or(v.sint_value.map(|i| i as f64))
            .or(v.uint_value.map(|i| i as f64))
            .unwrap()
    };
    layer
        .features
        .iter()
        .map(|f| {
            let mut ele = None;
            let mut level = None;
            for pair in f.tags.chunks(2) {
                let v = value(&layer.values[pair[1] as usize]);
                match layer.keys[pair[0] as usize].as_str() {
                    "ele" => ele = Some(v),
                    "level" => level = Some(v as i64),
                    other => panic!("unexpected key {other}"),
                }
            }
            (ele.unwrap(), level.unwrap())
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn health() {
    let (app, _) = service().await;
    let (status, _, body) = get_response(&app, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn below_lowest_threshold_is_empty_without_fetching() {
    let (app, fetches) = service().await;
    let (status, _, body) = get_response(&app, "/8/128/128").await;
    assert_eq!(status, StatusCode::OK);
    assert!(contours(&body).is_empty());
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn tile_headers() {
    let (app, _) = service().await;
    let (status, headers, _) = get_response(&app, "/11/1024/1024").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-type"],
        "application/vnd.mapbox-vector-tile"
    );
    assert_eq!(headers["cache-control"], "public, max-age=604800");
}

#[tokio::test(flavor = "multi_thread")]
async fn spacing_matches_the_style_at_each_zoom() {
    let (app, _) = service().await;
    for (z, minor, major) in [
        (9, 500.0, 2500.0),
        (10, 200.0, 1000.0),
        (11, 100.0, 500.0),
        (12, 50.0, 250.0),
        (13, 20.0, 100.0),
        (14, 10.0, 50.0),
        (16, 10.0, 50.0),
    ] {
        let xy = 1u32 << (z - 1);
        let (_, _, body) = get_response(&app, &format!("/{z}/{xy}/{xy}")).await;
        let lines = contours(&body);
        let eles: std::collections::BTreeSet<i64> = lines.iter().map(|l| l.0 as i64).collect();
        assert!(eles.len() > 1, "z{z}");
        assert!(
            eles.iter()
                .zip(eles.iter().skip(1))
                .any(|(a, b)| (b - a) as f64 == minor),
            "z{z}: no adjacent lines {minor} apart in {eles:?}"
        );
        for (ele, level) in lines {
            assert_eq!(ele % minor, 0.0, "z{z}");
            assert_eq!(level, (ele % major == 0.0) as i64, "z{z} ele {ele}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn imperial_multiplies_elevation() {
    let (app, _) = service().await;
    let (_, _, metric) = get_response(&app, "/13/4096/4096").await;
    let (_, _, imperial) = get_response(&app, "/13/4096/4096?units=imperial").await;
    let metric = contours(&metric);
    let imperial = contours(&imperial);
    let max = |lines: &[(f64, i64)]| lines.iter().map(|l| l.0).fold(f64::MIN, f64::max);
    assert!(imperial.len() > 3 * metric.len() - 10);
    assert!((max(&imperial) / max(&metric) - 3.28084).abs() < 0.05);
}

#[tokio::test(flavor = "multi_thread")]
async fn units_share_the_dem_cache() {
    let (app, fetches) = service().await;
    get_response(&app, "/13/4096/4096").await;
    let after_metric = fetches.load(Ordering::SeqCst);
    get_response(&app, "/13/4096/4096?units=imperial").await;
    assert!(after_metric > 0);
    assert_eq!(fetches.load(Ordering::SeqCst), after_metric);
}

#[tokio::test(flavor = "multi_thread")]
async fn tile_suffixes_are_accepted() {
    let (app, _) = service().await;
    let (_, _, plain) = get_response(&app, "/12/2048/2048").await;
    let (_, _, pbf) = get_response(&app, "/12/2048/2048.pbf").await;
    let (_, _, mvt) = get_response(&app, "/12/2048/2048.mvt").await;
    assert_eq!(plain, pbf);
    assert_eq!(plain, mvt);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_coordinates_are_rejected() {
    let (app, _) = service().await;
    for uri in [
        "/12/4096/0",
        "/12/0/4096",
        "/12/0/abc",
        "/12/0/1.png",
        "/31/0/0",
        "/300/0/0",
    ] {
        let (status, _, _) = get_response(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_dem_is_empty() {
    let (app, _) = service().await;
    let (status, _, body) = get_response(&app, &format!("/12/{}/1024", MISSING_X * 2)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(contours(&body).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn dem_errors_are_bad_gateway() {
    let (app, _) = service().await;
    let (status, headers, _) = get_response(&app, &format!("/12/{}/1024", BROKEN_X * 2)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(headers.get("cache-control").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_dem_is_bad_gateway() {
    let settings = Settings {
        dem_url_pattern: "http://127.0.0.1:1/{z}/{x}/{y}.png".to_string(),
        cache_size: 1,
        public_url: PUBLIC_URL.to_string(),
        attribution: DEFAULT_ATTRIBUTION.to_string(),
    };
    let (status, _, _) = get_response(&router(&settings), "/12/2048/2048").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

async fn get_tilejson(app: &Router) -> serde_json::Value {
    let (status, headers, body) = get_response(app, "/tilejson.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn tilejson_describes_the_contour_tiles() {
    let (app, fetches) = service().await;
    let tilejson = get_tilejson(&app).await;
    assert_eq!(tilejson["tilejson"], "3.0.0");
    assert_eq!(tilejson["scheme"], "xyz");
    assert_eq!(tilejson["attribution"], DEFAULT_ATTRIBUTION);
    assert_eq!(
        tilejson["tiles"],
        serde_json::json!([format!("{PUBLIC_URL}/{{z}}/{{x}}/{{y}}.mvt")])
    );
    assert_eq!(tilejson["minzoom"], 9);
    assert_eq!(tilejson["maxzoom"], 16);
    let layer = &tilejson["vector_layers"][0];
    assert_eq!(layer["id"], "contours");
    assert_eq!(
        layer["fields"],
        serde_json::json!({"ele": "Number", "level": "Number"})
    );
    assert_eq!(layer["minzoom"], 9);
    assert_eq!(layer["maxzoom"], 16);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn tilejson_tile_url_matches_the_tile_route() {
    let (app, _) = service().await;
    let tilejson = get_tilejson(&app).await;
    let minzoom = tilejson["minzoom"].as_u64().unwrap() as u32;
    let template = tilejson["tiles"][0].as_str().unwrap();
    let path = template.strip_prefix(PUBLIC_URL).unwrap();
    let uri = |z: u32| {
        let xy = (1u32 << z) / 2;
        path.replace("{z}", &z.to_string())
            .replace("{x}", &xy.to_string())
            .replace("{y}", &xy.to_string())
    };
    let (status, _, body) = get_response(&app, &uri(minzoom)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!contours(&body).is_empty());
    let (status, _, body) = get_response(&app, &uri(minzoom - 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(contours(&body).is_empty());
}

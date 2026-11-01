use dem_contour_service::{
    healthcheck, router, Settings, DEFAULT_ATTRIBUTION, DEFAULT_CACHE_SIZE, DEFAULT_DEM_URL_PATTERN,
};
use tokio::signal::unix::{signal, SignalKind};

async fn shutdown_signal() {
    let mut terminate = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(if healthcheck(&port) { 0 } else { 1 });
    }
    let settings = Settings {
        dem_url_pattern: std::env::var("DEM_URL_PATTERN")
            .unwrap_or_else(|_| DEFAULT_DEM_URL_PATTERN.to_string()),
        cache_size: std::env::var("DEM_CACHE_SIZE")
            .ok()
            .map(|v| {
                v.parse()
                    .expect("DEM_CACHE_SIZE must be a positive integer")
            })
            .unwrap_or(DEFAULT_CACHE_SIZE),
        public_url: std::env::var("PUBLIC_URL")
            .unwrap_or_else(|_| format!("http://localhost:{port}")),
        attribution: std::env::var("ATTRIBUTION")
            .unwrap_or_else(|_| DEFAULT_ATTRIBUTION.to_string()),
    };
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("failed to bind port");
    axum::serve(listener, router(&settings))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

# dem-contour-service

An HTTP service that generates contour-line vector tiles (MVT) on the fly from Terrarium raster-DEM tiles.

It uses [maplibre-contour-rs](https://github.com/mapeak-com/maplibre-contour-rs) with the DEM configuration of the Mapeak Android and iOS apps (`CarContourTilesProvider` in [IsraelHikingMap/Site](https://github.com/IsraelHikingMap/Site)) and the contour spacing of the [mapeak-hike](https://github.com/IsraelHikingMap/VectorMap/blob/master/Styles/mapeak-hike.json) style:

| Setting | Value |
|---|---|
| DEM encoding | Terrarium |
| DEM max zoom | 11 |
| Overzoom | 1 |
| Thresholds | `9*500*2500~10*200*1000~11*100*500~12*50*250~13*20*100~14*10*50` |
| Layer / attributes | `contours` / `ele`, `level` |
| Multiplier | 1.0 (metric), 3.28084 (imperial) |

## API

| Request | Response |
|---|---|
| `GET /{z}/{x}/{y}` | Contour MVT in meters. `{y}` may end with `.pbf` or `.mvt`. |
| `GET /{z}/{x}/{y}?units=imperial` | Contour MVT in feet. |
| `GET /tilejson.json` | TileJSON 3.0.0 for the metric tiles, zoom 9-16, tiles at `{PUBLIC_URL}/{z}/{x}/{y}.mvt`. |
| `GET /health` | `ok` |

- Tiles below zoom 9 are empty.
- A DEM tile answered with 404 counts as no data and yields an empty tile.
- Other DEM failures return 502.
- Successful tiles carry `Cache-Control: public, max-age=604800`.

## Configuration

| Variable | Default |
|---|---|
| `PORT` | `8080` |
| `DEM_URL_PATTERN` | `https://mapeak.com/vector/data/raster-dem/{z}/{x}/{y}.webp` |
| `DEM_CACHE_SIZE` | `64` decoded DEM tiles, shared by both unit systems |
| `PUBLIC_URL` | `http://localhost:{PORT}`, the external base URL of the tiles in the TileJSON |
| `ATTRIBUTION` | `<a href='https://mapterhorn.com/attribution'>© Mapterhorn</a>` |

## Running

```bash
docker run -p 8080:8080 ghcr.io/israelhikingmap/dem-contour-service
```

With Docker Compose:

```yaml
services:
  dem-contour-service:
    image: ghcr.io/israelhikingmap/dem-contour-service:latest
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      DEM_URL_PATTERN: https://mapeak.com/vector/data/raster-dem/{z}/{x}/{y}.webp
      DEM_CACHE_SIZE: 64
```

As a MapLibre vector source:

```json
{
  "type": "vector",
  "url": "http://localhost:8080/tilejson.json"
}
```

## Development

```bash
cargo test
cargo run --release
```

The GitHub workflow runs the tests, smoke-tests the image against the live DEM, and publishes it to `ghcr.io` on pushes to `main` and `v*` tags.

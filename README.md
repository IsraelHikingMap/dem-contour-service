# dem-contour-service

An HTTP service that generates contour-line vector tiles (MVT) on the fly from Terrarium raster-DEM tiles.

It uses [maplibre-contour-rs](https://github.com/mapeak-com/maplibre-contour-rs) with the same configuration as the Mapeak Android and iOS apps (`CarContourTilesProvider` in [IsraelHikingMap/Site](https://github.com/IsraelHikingMap/Site)):

| Setting | Value |
|---|---|
| DEM encoding | Terrarium |
| DEM max zoom | 11 |
| Overzoom | 1 |
| Thresholds | `11*200*1000~12*10*100~13*10*100~14*10*100~15*10*100` |
| Layer / attributes | `contours` / `ele`, `level` |
| Multiplier | 1.0 (metric), 3.28084 (imperial) |

## API

| Request | Response |
|---|---|
| `GET /{z}/{x}/{y}` | Contour MVT in meters. `{y}` may end with `.pbf` or `.mvt`. |
| `GET /{z}/{x}/{y}?units=imperial` | Contour MVT in feet. |
| `GET /health` | `ok` |

- Tiles below zoom 11 are empty.
- A DEM tile answered with 404 counts as no data and yields an empty tile.
- Other DEM failures return 502.
- Successful tiles carry `Cache-Control: public, max-age=604800`.

## Configuration

| Variable | Default |
|---|---|
| `PORT` | `8080` |
| `DEM_URL_PATTERN` | `https://mapeak.com/vector/data/raster-dem/{z}/{x}/{y}.webp` |
| `DEM_CACHE_SIZE` | `64` decoded DEM tiles, shared by both unit systems |

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
  "tiles": ["http://localhost:8080/{z}/{x}/{y}"],
  "maxzoom": 16
}
```

## Development

```bash
cargo test
cargo run --release
```

The GitHub workflow runs the tests, smoke-tests the image against the live DEM, and publishes it to `ghcr.io` on pushes to `main` and `v*` tags.

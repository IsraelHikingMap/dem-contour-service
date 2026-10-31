FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
RUN cargo build --release --locked

FROM build AS test
RUN cargo test --release --locked

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/dem-contour-service /usr/local/bin/dem-contour-service
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/dem-contour-service"]

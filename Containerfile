FROM docker.io/library/rust:1.88.0-bookworm AS build

RUN apt-get update \
    && apt-get install --yes --no-install-recommends clang cmake ninja-build \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release --package ggreddit

FROM docker.io/library/debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 ggreddit \
    && useradd --no-create-home --uid 10001 --gid 10001 --shell /usr/sbin/nologin ggreddit \
    && install --directory --owner=ggreddit --group=ggreddit /var/cache/ggreddit

COPY --from=build /src/target/release/ggreddit /usr/local/bin/ggreddit
COPY container.toml /etc/ggreddit/config.toml
USER ggreddit:ggreddit
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/ggreddit"]
CMD ["--config", "/etc/ggreddit/config.toml"]

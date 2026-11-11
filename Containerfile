FROM docker.io/library/rust:1.88.0-bookworm AS build

RUN apt-get update \
    && apt-get install --yes --no-install-recommends clang cmake ninja-build \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release --package neddit

FROM docker.io/library/debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 neddit \
    && useradd --no-create-home --uid 10001 --gid 10001 --shell /usr/sbin/nologin neddit \
    && install --directory --owner=neddit --group=neddit /var/cache/neddit

COPY --from=build /src/target/release/neddit /usr/local/bin/neddit
COPY container.toml /etc/neddit/config.toml
USER neddit:neddit
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/neddit"]
CMD ["--config", "/etc/neddit/config.toml"]

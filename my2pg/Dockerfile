FROM docker.io/library/rust:1.99.0-bookworm@sha256:59037199c44290f2befcdd58dcc540164763fc296950255aaefeef096a1866b0 AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/ ./src/
RUN cargo build --locked --release --bin my2pg

FROM scratch AS release-binary
COPY --from=builder /src/target/release/my2pg /my2pg

FROM docker.io/library/debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251

ARG VCS_REF=unknown
LABEL org.opencontainers.image.title="my2pg" \
      org.opencontainers.image.description="MySQL to PostgreSQL migration CLI" \
      org.opencontainers.image.licenses="PostgreSQL" \
      org.opencontainers.image.revision="$VCS_REF"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 10001 my2pg \
 && useradd --uid 10001 --gid my2pg --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin my2pg \
 && install -d -o my2pg -g my2pg /work

COPY --from=builder /src/target/release/my2pg /usr/local/bin/my2pg
COPY LICENSE /usr/share/licenses/my2pg/LICENSE
COPY THIRD-PARTY-NOTICES.md /usr/share/doc/my2pg/THIRD-PARTY-NOTICES.md

WORKDIR /work
USER 10001:10001
ENTRYPOINT ["my2pg"]

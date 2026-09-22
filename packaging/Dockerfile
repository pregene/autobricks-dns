# syntax=docker/dockerfile:1
FROM ubuntu:22.04 AS toolchain
ARG DEBIAN_FRONTEND=noninteractive
ARG RUST_VERSION=1.85.1
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl debhelper pkg-config \
    systemd adduser python3 file && rm -rf /var/lib/apt/lists/*
ENV PATH=/root/.cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain "$RUST_VERSION" \
    && rm /tmp/rustup.sh

FROM toolchain AS builder
WORKDIR /work/autobricks-dns
COPY Cargo.toml Cargo.lock VERSION build.rs README.md INSTALL.md LICENSE ./
COPY src/ src/
COPY config/ config/
COPY debian/ debian/
COPY packaging/README.md packaging/README.md
RUN chmod 0755 debian/rules debian/postinst debian/prerm debian/postrm \
    && dpkg-buildpackage -us -uc -b \
    && mkdir /artifacts \
    && cp /work/*.deb /artifacts/

FROM ubuntu:22.04 AS verified
COPY --from=builder /artifacts/ /artifacts/
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        python3 /artifacts/autobricks-dns_*.deb
COPY packaging/test-deb.sh /test-deb.sh
RUN sh /test-deb.sh /artifacts/autobricks-dns_*.deb

FROM scratch AS artifact
COPY --from=verified /artifacts/ /

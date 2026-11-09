FROM rust:1-slim-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p pdfword-web

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        fontconfig \
        fonts-dejavu-core \
        libreoffice-writer \
        tar \
    && rm -rf /var/lib/apt/lists/*

# Pdfium is a runtime dependency for PDF -> DOCX extraction.
# Prebuilt binaries: https://github.com/bblanchon/pdfium-binaries
ARG PDFIUM_VERSION=latest
RUN set -eux; \
    mkdir -p /opt/pdfium; \
    url="https://github.com/bblanchon/pdfium-binaries/releases/${PDFIUM_VERSION}/download/pdfium-linux-x64.tgz"; \
    if [ "$PDFIUM_VERSION" = "latest" ]; then \
        url="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz"; \
    fi; \
    curl -fsSL "$url" | tar -xz -C /opt/pdfium; \
    test -f /opt/pdfium/lib/libpdfium.so

ENV PDFIUM_PATH=/opt/pdfium/lib
ENV SOFFICE_PATH=/usr/bin/soffice
ENV BIND_ADDR=0.0.0.0:8080
ENV WORK_DIR=/data

COPY --from=builder /app/target/release/pdfword /usr/local/bin/pdfword
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY crates/web/static /app/crates/web/static
WORKDIR /app

RUN chmod +x /usr/local/bin/docker-entrypoint.sh \
    && mkdir -p /data /usr/share/fonts/truetype/custom \
    && chown nobody:nogroup /data

EXPOSE 8080

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["pdfword"]

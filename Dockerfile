# syntax=docker/dockerfile:1
FROM debian:stable-slim AS runtime
RUN apt-get update && apt-get install --no-install-recommends -y \
    ca-certificates curl git nodejs npm which unzip xz-utils bzip2 \
    && apt-get clean && rm -rf /var/lib/apt/lists/*
COPY target/release/ws /usr/local/bin/ws
WORKDIR /workspace
ENTRYPOINT ["ws"]
CMD ["--help"]

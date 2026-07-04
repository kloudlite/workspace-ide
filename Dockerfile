# syntax=docker/dockerfile:1
FROM debian:stable-slim AS runtime
RUN apt-get update && apt-get install --no-install-recommends -y \
    ca-certificates curl git which unzip xz-utils bzip2 \
    && apt-get clean && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 1000 karthik && useradd -u 1000 -g 1000 -d /home/karthik -s /bin/bash -m karthik
COPY target/release/ws /usr/local/bin/ws
WORKDIR /workspace
USER karthik
ENV HOME=/home/karthik
ENTRYPOINT ["ws"]
CMD ["--help"]

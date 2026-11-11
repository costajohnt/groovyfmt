# The static musl release binary on an empty base. Build from a context made by
# packaging/docker/context.sh, which verifies and unpacks the release archives:
#   packaging/docker/context.sh <archives-dir> ctx
#   docker buildx build --platform linux/amd64,linux/arm64 -f Dockerfile ctx
# Usage: docker run --rm -v "$PWD:/work" ghcr.io/costajohnt/groovyfmt --check
FROM scratch
ARG TARGETARCH
LABEL org.opencontainers.image.source=https://github.com/costajohnt/groovyfmt \
      org.opencontainers.image.description="Formatter and linter for Groovy, Gradle build scripts, and Jenkinsfiles" \
      org.opencontainers.image.licenses=MIT
COPY LICENSE /usr/share/doc/groovyfmt/LICENSE
COPY ${TARGETARCH}/groovyfmt /usr/local/bin/groovyfmt
ENV PATH=/usr/local/bin
WORKDIR /work
ENTRYPOINT ["groovyfmt"]

# ghcr.io/hew3d/hew-relay — a scratch wrap of the static musl binary that
# .github/workflows/release.yml builds for linux/amd64 and linux/arm64. The
# build context holds the prebuilt binaries at
# dist/<TARGETPLATFORM>/hew-relay (e.g. dist/linux/amd64/hew-relay); no
# compilation happens here, so a multi-platform build needs no emulation.
#
# Serves plain HTTP on 8787 inside the container; put nginx (or any TLS
# terminator) in front — see docs/SELF_HOSTING.md, Option B.
FROM scratch
ARG TARGETPLATFORM
COPY dist/${TARGETPLATFORM}/hew-relay /hew-relay
# Inside a container the loopback default is unreachable; every other
# setting keeps hew-relay's own default and is overridable with the
# HEW_RELAY_* environment (docs/SELF_HOSTING.md).
ENV HEW_RELAY_LISTEN=0.0.0.0:8787
EXPOSE 8787
USER 65534:65534
ENTRYPOINT ["/hew-relay"]

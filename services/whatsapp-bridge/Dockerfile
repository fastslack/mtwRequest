# ── Stage 1: Go build ─────────────────────────────────────────────────
# whatsmeow@latest requires Go >= 1.25. Pinning to a specific minor is
# cheap and avoids surprise upgrades breaking the build.
FROM golang:1.25-alpine AS build

WORKDIR /src

# git is needed by `go get` to fetch modules from git.mau.fi; build-base
# is needed by a couple of transitive cgo-free packages at link time.
RUN apk add --no-cache git build-base

# Copy only the sources — go.mod starts minimal and dependencies are
# resolved at build time via `go get @latest`, so the image always picks
# up the newest whatsmeow (which tracks the WhatsApp Multi-Device
# protocol drift, so pinning is actually harmful long-term).
COPY . .

RUN go get go.mau.fi/whatsmeow@latest \
 && go get google.golang.org/protobuf@latest \
 && go get modernc.org/sqlite@latest \
 && go mod tidy

# Static binary, stripped.
ENV CGO_ENABLED=0
RUN go build -trimpath -ldflags="-s -w" -o /out/whatsapp-bridge .

# ── Stage 2: Runtime ──────────────────────────────────────────────────
FROM alpine:3.20

RUN apk add --no-cache ca-certificates tzdata && \
    addgroup -S mtw && adduser -S -G mtw mtw && \
    mkdir -p /var/run/mtw-whatsapp /var/lib/mtw-whatsapp && \
    chown -R mtw:mtw /var/run/mtw-whatsapp /var/lib/mtw-whatsapp

COPY --from=build /out/whatsapp-bridge /usr/local/bin/whatsapp-bridge

USER mtw

ENV MTW_WHATSAPP_SOCKET=/var/run/mtw-whatsapp/whatsapp.sock \
    MTW_WHATSAPP_DB=/var/lib/mtw-whatsapp/session.db

VOLUME ["/var/run/mtw-whatsapp", "/var/lib/mtw-whatsapp"]

ENTRYPOINT ["/usr/local/bin/whatsapp-bridge"]

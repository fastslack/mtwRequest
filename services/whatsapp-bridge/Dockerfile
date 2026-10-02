# ── Stage 1: Go build ─────────────────────────────────────────────────
# whatsmeow requires Go >= 1.26.
FROM golang:1.26-alpine AS build

WORKDIR /src

RUN apk add --no-cache git

# Dependencies are pinned in go.mod/go.sum on purpose: a build must be
# reproducible. whatsmeow tracks WhatsApp Multi-Device protocol drift, so
# bump it deliberately with `go get go.mau.fi/whatsmeow@<version>` followed
# by `go mod tidy`, and commit the updated go.mod/go.sum.
COPY go.mod go.sum ./
RUN go mod download

COPY . .

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

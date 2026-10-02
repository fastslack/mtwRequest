package main

import (
	"context"
	"time"
)

const maxChats = 200

func (b *bridge) listChats(id string, limit int) {
	if b.sess.state() != "connected" {
		b.emitErr(id, "not_connected", "whatsapp is not linked")
		return
	}
	if limit <= 0 || limit > maxChats {
		limit = 50
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	items, err := b.lk.Chats(ctx)
	if err != nil {
		b.emitErr(id, "list_failed", err.Error())
		return
	}
	if len(items) > limit {
		items = items[:limit]
	}
	b.emit(outMsg{"type": "chats", "id": id, "items": items})
}

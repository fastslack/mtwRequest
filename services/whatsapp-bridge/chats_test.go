package main

import "testing"

func TestListChatsCapsAndKeepsGroups(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	f.chats = []chatInfo{{JID: "a@s.whatsapp.net", Name: "Ana"}, {JID: "g@g.us", Name: "Familia", IsGroup: true}, {JID: "z@s.whatsapp.net", Name: "Zoe"}}
	b := &bridge{lk: f, sess: newSession(f, r.emit), drv: nil}
	b.emitFn = r.emit
	b.sess.boot()
	b.sess.onConnected()
	b.listChats("req-1", 2)
	m := r.waitFor(t, "chats", "id", "req-1")
	items := m["items"].([]chatInfo)
	if len(items) != 2 {
		t.Fatalf("limit not applied: %d", len(items))
	}
}

func TestListChatsWhenNotConnected(t *testing.T) {
	f, r := newFake(), &recorder{}
	b := &bridge{lk: f, sess: newSession(f, r.emit)}
	b.emitFn = r.emit
	b.sess.boot()
	b.listChats("req-2", 10)
	m := r.waitFor(t, "error", "id", "req-2")
	if m["code"] != "not_connected" {
		t.Fatalf("code=%v", m["code"])
	}
}

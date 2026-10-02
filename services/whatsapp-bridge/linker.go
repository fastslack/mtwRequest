package main

import (
	"context"
	"sort"

	"go.mau.fi/whatsmeow"
)

type qrItem struct {
	Event string
	Code  string
}

type chatInfo struct {
	JID     string `json:"jid"`
	Name    string `json:"name"`
	IsGroup bool   `json:"is_group"`
	LastTS  int64  `json:"last_ts"`
}

// linker is the slice of whatsmeow the linking flow needs; tests fake it.
type linker interface {
	HasSession() bool
	Connect() error
	Disconnect()
	QRChannel(ctx context.Context) (<-chan qrItem, error)
	PairPhone(ctx context.Context, phone string) (string, error)
	Logout(ctx context.Context) error
	OwnJID() string
	Chats(ctx context.Context) ([]chatInfo, error)
}

type waLinker struct{ c *whatsmeow.Client }

func (w *waLinker) HasSession() bool { return w.c.Store.ID != nil }
func (w *waLinker) Connect() error   { return w.c.Connect() }
func (w *waLinker) Disconnect()      { w.c.Disconnect() }
func (w *waLinker) OwnJID() string {
	if w.c.Store.ID == nil {
		return ""
	}
	return w.c.Store.ID.String()
}
func (w *waLinker) Logout(ctx context.Context) error { return w.c.Logout(ctx) }

func (w *waLinker) QRChannel(ctx context.Context) (<-chan qrItem, error) {
	src, err := w.c.GetQRChannel(ctx)
	if err != nil {
		return nil, err
	}
	out := make(chan qrItem, 4)
	go func() {
		defer close(out)
		for it := range src {
			out <- qrItem{Event: it.Event, Code: it.Code}
		}
	}()
	return out, nil
}

func (w *waLinker) PairPhone(ctx context.Context, phone string) (string, error) {
	return w.c.PairPhone(ctx, phone, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
}

func (w *waLinker) Chats(ctx context.Context) ([]chatInfo, error) {
	var out []chatInfo
	contacts, err := w.c.Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil, err
	}
	for jid, ci := range contacts {
		name := ci.FullName
		if name == "" {
			name = ci.PushName
		}
		if name == "" {
			name = ci.BusinessName
		}
		out = append(out, chatInfo{JID: jid.String(), Name: name})
	}
	groups, err := w.c.GetJoinedGroups(ctx)
	if err != nil {
		return nil, err
	}
	for _, g := range groups {
		out = append(out, chatInfo{JID: g.JID.String(), Name: g.GroupName.Name, IsGroup: true})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}

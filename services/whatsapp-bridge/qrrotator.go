package main

import (
	"context"
	"strings"
	"time"
)

// qrFirstTimeout and qrRestTimeout match whatsmeow's own QR rotation timing
// in qrchan.go's emitQRs: the first code (of whatsapp's usual 6-code batch)
// stays valid for 60s, every one after for 20s.
const (
	qrFirstTimeout = 60 * time.Second
	qrRestTimeout  = 20 * time.Second
)

// advRotation mirrors events.RotateADVSecret: the server can replace the ADV
// secret mid-attempt (pair.go's rotateADVSecret, dispatched on a
// companion_reg_refresh/pair-device-rotate-qr notification). Every QR code
// embeds the ADV secret as one of its comma-separated fields (makeQRData),
// and the server's HMAC check on pairing (handlePair) is against whichever
// secret is current — so a code generated before a rotation fails pairing
// with hmac-mismatch if presented after. Old/New are both base64, the same
// encoding embedded in the QR string, so a literal substring replace (same
// as whatsmeow's own qrchan.go) is all patching needs.
type advRotation struct {
	Old string
	New string
}

// qrRotator owns the client-side timing of QR code rotation, decoupled from
// whatsmeow's own per-attempt event handler (see linker.go for why). `after`
// is injectable so tests can run the whole sequence without real waits.
type qrRotator struct {
	after func(time.Duration) <-chan time.Time
}

func newQRRotator() *qrRotator {
	return &qrRotator{after: time.After}
}

// run emits codes on out in order, each with whatsmeow's own QR timing, then
// a final {Event: "timeout"} once they're exhausted. It returns early,
// without emitting "timeout", as soon as ctx is done. out is never closed by
// run — the caller owns that, since run is not the only possible sender into
// a shared per-attempt channel (see waLinker.runAttempt).
//
// rotate delivers advRotation items (nil is fine — it then just never fires,
// same as whatsmeow's own rotateAdv channel when nothing rotates). On a
// rotation, the just-sent code and every remaining one are patched in place
// and the (patched) current code is re-emitted immediately, with a fresh
// wait — this exactly mirrors qrchan.go's emitQRs: its rotateAdv case patches
// `nextCode`+`codes` into a new slice with the patched current code back at
// the front, then falls through to the top of its for loop, which re-pops
// and re-sends that same code before waiting again.
func (r *qrRotator) run(ctx context.Context, codes []string, out chan<- qrItem, rotate <-chan advRotation) {
	for len(codes) > 0 {
		timeout := qrRestTimeout
		if len(codes) == 6 {
			timeout = qrFirstTimeout
		}
		code := codes[0]
		codes = codes[1:]
		select {
		case out <- qrItem{Event: "code", Code: code}:
		case <-ctx.Done():
			return
		}
		select {
		case <-r.after(timeout):
			// advance to the next code; nothing more to do this iteration.
		case rot := <-rotate:
			patched := make([]string, len(codes)+1)
			patched[0] = strings.Replace(code, rot.Old, rot.New, 1)
			for i, c := range codes {
				patched[i+1] = strings.Replace(c, rot.Old, rot.New, 1)
			}
			codes = patched // loop re-sends patched[0] immediately, with a fresh wait
		case <-ctx.Done():
			return
		}
	}
	select {
	case out <- qrItem{Event: "timeout"}:
	case <-ctx.Done():
	}
}

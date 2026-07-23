package contextgraph

import (
	"bufio"
	"encoding/json"
	"os"
	"strings"
)

// Provider is a Context Graph Protocol provider. Query is mandatory. To answer
// context/verify, also implement Verifier; a provider that does not is treated
// as unable to vouch for its frames, and the host re-queries.
type Provider interface {
	// Info reports identity and data-flow posture at handshake.
	Info() ProviderInfo
	// Capabilities reports what this provider can do, negotiated at handshake.
	Capabilities() Capabilities
	// Query answers a retrieval request with budgeted, provenance-carrying frames.
	Query(query ContextQuery) ContextQueryResult
}

// Verifier is the optional context/verify surface. Implement it alongside
// Provider to revalidate frames a host already holds (identities only — never
// bodies).
type Verifier interface {
	Verify(request VerifyRequest) VerifyResponse
}

type incomingEnvelope struct {
	Type    string         `json:"type"`
	ID      *string        `json:"id"`
	Query   *ContextQuery  `json:"query"`
	Request *VerifyRequest `json:"request"`
}

type handshakeAck struct {
	Type            string       `json:"type"`
	ProtocolVersion string       `json:"protocol_version"`
	Provider        ProviderInfo `json:"provider"`
	Capabilities    Capabilities `json:"capabilities"`
}

type framesReply struct {
	Type   string             `json:"type"`
	Result ContextQueryResult `json:"result"`
	ID     *string            `json:"id,omitempty"`
}

type verifiedReply struct {
	Type     string         `json:"type"`
	Response VerifyResponse `json:"response"`
}

type errorReply struct {
	Type    string `json:"type"`
	Code    string `json:"code,omitempty"`
	Message string `json:"message"`
}

func writeEnvelope(w *bufio.Writer, envelope any) {
	data, err := json.Marshal(envelope)
	if err != nil {
		return
	}
	_, _ = w.Write(data)
	_ = w.WriteByte('\n')
	_ = w.Flush()
}

// RunStdioProvider runs provider as a stdio child process — the shape the
// reference host and the conformance suite drive. It reads one envelope per
// line from stdin and writes one per line to stdout until a shutdown (or EOF),
// staying alive with a typed error on a malformed line rather than crashing.
func RunStdioProvider(provider Provider) {
	reader := bufio.NewReader(os.Stdin)
	writer := bufio.NewWriter(os.Stdout)
	for {
		line, err := reader.ReadString('\n')
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			handleLine(provider, trimmed, writer)
		}
		if err != nil { // io.EOF or a read error: the host is gone.
			break
		}
	}
}

func handleLine(provider Provider, line string, w *bufio.Writer) {
	var envelope incomingEnvelope
	if err := json.Unmarshal([]byte(line), &envelope); err != nil {
		// Malformed line: stay alive and say so with a code, don't crash.
		writeEnvelope(w, errorReply{
			Type:    "error",
			Code:    "bad_request",
			Message: "line was not a valid CGP envelope",
		})
		return
	}

	switch envelope.Type {
	case "handshake":
		writeEnvelope(w, handshakeAck{
			Type:            "handshake_ack",
			ProtocolVersion: ProtocolVersion,
			Provider:        provider.Info(),
			Capabilities:    provider.Capabilities(),
		})
	case "query":
		if envelope.Query == nil {
			return
		}
		reply := framesReply{Type: "frames", Result: provider.Query(*envelope.Query)}
		// Echo the correlation id so the host can match reply to request (H4).
		reply.ID = envelope.ID
		writeEnvelope(w, reply)
	case "verify":
		if envelope.Request == nil {
			return
		}
		var response VerifyResponse
		if verifier, ok := provider.(Verifier); ok {
			response = verifier.Verify(*envelope.Request)
		} else {
			// No verify support: vouch for nothing; the host re-queries.
			verdicts := make([]FrameVerdict, len(envelope.Request.Frames))
			for i, frame := range envelope.Request.Frames {
				verdicts[i] = FrameVerdict{Frame: frame, Status: "unknown"}
			}
			response = VerifyResponse{Verdicts: verdicts}
		}
		writeEnvelope(w, verifiedReply{Type: "verified", Response: response})
	case "shutdown":
		os.Exit(0)
		// handshake_ack / frames / verified / error are host->provider-invalid; ignore.
	}
}

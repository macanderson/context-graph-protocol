package contextgraph

// The HTTP adapter: host a Provider behind a single POST endpoint, speaking the
// same Context Graph Protocol wire as RunStdioProvider — the "streamable HTTP"
// transport (SPEC.md §3). The host POSTs one envelope as the request body and
// expects one envelope back as the response body; RespondToBody is that
// request/response state machine, and Handler wraps it as a net/http.Handler.
//
// The one deliberate difference from stdio: an HTTP provider is a long-lived
// server reached by many independent hosts, so a shutdown envelope ends that
// exchange — it never calls os.Exit. (contextgraph-inspect http in fact
// handshakes and shuts down twice per run: once to probe, once to run the
// conformance suite. A server that exited on the first shutdown could not
// answer the second handshake.)

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
)

// RespondToBody drives one request body through provider and returns the HTTP
// status plus the response payload to send back. A shutdown (and any
// host->provider-invalid envelope) yields 204 with a nil payload; a body that
// is not a valid CGP envelope yields 400 with a coded error envelope rather than
// a panic — the HTTP mirror of the stdio malformed-input-tolerance guarantee.
func RespondToBody(provider Provider, body []byte) (int, []byte) {
	var envelope incomingEnvelope
	if err := json.Unmarshal(body, &envelope); err != nil {
		payload, _ := json.Marshal(errorReply{
			Type:    "error",
			Code:    "bad_request",
			Message: "request body was not a valid CGP envelope",
		})
		return http.StatusBadRequest, payload
	}

	reply := handleEnvelope(provider, envelope)
	if reply == nil {
		return http.StatusNoContent, nil
	}
	payload, err := json.Marshal(reply)
	if err != nil {
		fallback, _ := json.Marshal(errorReply{Type: "error", Code: "internal", Message: err.Error()})
		return http.StatusInternalServerError, fallback
	}
	return http.StatusOK, payload
}

// handleEnvelope is the transport-free protocol state machine: one request
// envelope in, the one reply envelope out (or nil for a shutdown / ignored
// input). It mirrors handleLine exactly — including echoing a query's
// correlation id (H4) and turning a ProviderError into a coded error envelope
// (§E1) — minus the process lifecycle.
func handleEnvelope(provider Provider, envelope incomingEnvelope) any {
	switch envelope.Type {
	case "handshake":
		return handshakeAck{
			Type:            "handshake_ack",
			ProtocolVersion: ProtocolVersion,
			Provider:        provider.Info(),
			Capabilities:    provider.Capabilities(),
		}
	case "query":
		if envelope.Query == nil {
			return nil
		}
		result, err := provider.Query(*envelope.Query)
		if err != nil {
			// A deliberate, coded refusal of a request the provider can't
			// honestly serve (§E1): an error envelope, not frames.
			reply := errorReply{Type: "error", Message: err.Error(), ID: envelope.ID}
			var pe ProviderError
			if errors.As(err, &pe) {
				reply.Code = pe.Code
			}
			return reply
		}
		// Echo the correlation id so the host can match reply to request (H4).
		return framesReply{Type: "frames", Result: result, ID: envelope.ID}
	case "verify":
		if envelope.Request == nil {
			return nil
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
		return verifiedReply{Type: "verified", Response: response}
	default:
		// shutdown ends the exchange but keeps the server alive; handshake_ack /
		// frames / verified / error are host->provider-invalid. Neither replies.
		return nil
	}
}

// Handler returns a net/http.Handler that answers the whole CGP protocol on one
// endpoint, so the zero-config path is:
//
//	http.ListenAndServe("127.0.0.1:8789", contextgraph.Handler(provider))
//
// It reads the raw request body itself, so it needs no middleware; mount it
// under any router (chi, gorilla/mux, http.ServeMux) at whatever path you like.
func Handler(provider Provider) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			payload, _ := json.Marshal(errorReply{
				Type:    "error",
				Code:    "bad_request",
				Message: "could not read request body",
			})
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusBadRequest)
			_, _ = w.Write(payload)
			return
		}
		status, payload := RespondToBody(provider, body)
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		if payload != nil {
			_, _ = w.Write(payload)
		}
	})
}

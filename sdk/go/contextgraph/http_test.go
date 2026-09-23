package contextgraph

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// The HTTP transport is a second copy of the same state machine, so it needs
// the same guarantees proved against it independently: an empty answer that
// serializes as `[]`, and a ProviderError code that survives either spelling.

func TestRespondToBodyEmptyResultSerializesFramesAsArray(t *testing.T) {
	provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
		return ContextQueryResult{}, nil
	})

	status, payload := RespondToBody(provider, []byte(queryLine))

	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200", status)
	}
	if !strings.Contains(string(payload), `"frames":[]`) {
		t.Fatalf("body does not carry an empty frames array: %s", payload)
	}
	if strings.Contains(string(payload), `"frames":null`) {
		t.Fatalf("body carries frames:null, which a conforming host rejects: %s", payload)
	}
}

func TestRespondToBodyNilVerdictsSerializeAsArray(t *testing.T) {
	provider := verifyingProvider{
		queryFunc: func(ContextQuery) (ContextQueryResult, error) { return ContextQueryResult{}, nil },
		verify:    func(VerifyRequest) VerifyResponse { return VerifyResponse{} },
	}

	_, payload := RespondToBody(provider, []byte(`{"type":"verify","request":{"frames":[]}}`))

	if !strings.Contains(string(payload), `"verdicts":[]`) {
		t.Fatalf("body does not carry an empty verdicts array: %s", payload)
	}
}

func TestRespondToBodyProviderErrorCodeSurvivesEverySpelling(t *testing.T) {
	for _, spelling := range errorSpellings {
		t.Run(spelling.name, func(t *testing.T) {
			provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
				return ContextQueryResult{}, spelling.err
			})

			_, payload := RespondToBody(provider, []byte(queryLine))

			var got errorReply
			if err := json.Unmarshal(payload, &got); err != nil {
				t.Fatalf("body was not a JSON envelope: %v", err)
			}
			if got.Type != "error" {
				t.Fatalf("reply type = %q, want error", got.Type)
			}
			if got.Code != spelling.code {
				t.Fatalf("reply code = %q, want %q", got.Code, spelling.code)
			}
		})
	}
}

// End to end through the net/http.Handler, so the guarantee is proved on the
// bytes a real host actually receives rather than on RespondToBody's return.
func TestHandlerServesEmptyFramesAsArray(t *testing.T) {
	provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
		return ContextQueryResult{}, nil
	})
	server := httptest.NewServer(Handler(provider))
	defer server.Close()

	response, err := http.Post(server.URL, "application/json", strings.NewReader(queryLine))
	if err != nil {
		t.Fatalf("post: %v", err)
	}
	defer func() { _ = response.Body.Close() }()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatalf("read body: %v", err)
	}
	if !strings.Contains(string(body), `"frames":[]`) {
		t.Fatalf("served body does not carry an empty frames array: %s", body)
	}
}

// The HTTP mirror of TestAMalformedRequestIsAnsweredBadRequest: a payload-less
// envelope used to get 204 No Content, so a host learned nothing about why its
// request produced no answer.
func TestRespondToBodyAnswersAPayloadlessEnvelopeBadRequest(t *testing.T) {
	provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
		t.Fatal("the provider must not be called for a request with no payload")
		return ContextQueryResult{}, nil
	})
	for _, body := range []string{`{"type":"query","id":"q1"}`, `{"type":"verify"}`} {
		status, payload := RespondToBody(provider, []byte(body))
		var got errorReply
		if err := json.Unmarshal(payload, &got); err != nil {
			t.Fatalf("%s: reply was not a JSON envelope (status %d): %v", body, status, err)
		}
		if got.Type != "error" || got.Code != "bad_request" {
			t.Fatalf("%s: reply = %+v, want an error with code bad_request", body, got)
		}
	}
}

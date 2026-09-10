"""The Python stdio and HTTP runtimes' failure contract.

``SPEC.md`` §11 R1: a provider MUST NOT crash on a malformed line or bad request,
and SHOULD reply ``error`` with a code. Every assertion here is about a reply
that *exists* and a process that is *still running*. Before the fix, three of
these inputs killed the loop outright (``AttributeError`` on a non-object line,
``KeyError`` on a missing payload) and a fourth -- an unexpected exception from
``provider.query`` -- propagated out of the loop the same way, losing every
correlation id already in flight.

Written against :mod:`unittest` rather than pytest so it runs on a bare
``python3`` with nothing installed, which is the same promise the SDK makes to
its users. ``python3 -m pytest`` collects it too.
"""

from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contextgraph_sdk.http import handle_envelope, respond_to_body  # noqa: E402
from contextgraph_sdk.provider import ProviderError, run_stdio_provider  # noqa: E402

INFO = {
    "name": "test-provider",
    "version": "0.0.0",
    "data_flow": {
        "reads": True,
        "writes": False,
        "egress": False,
        "egress_scopes": ["local-only"],
    },
}
CAPS = {"query": {"kinds": ["doc"]}, "correlation": True, "verify": True}

QUERY_LINE = json.dumps(
    {"type": "query", "id": "q1", "query": {"goal": "g", "max_frames": 8, "max_tokens": 99}}
)
VERIFY_LINE = json.dumps(
    {"type": "verify", "id": "v1", "request": {"frames": [{"provider_id": "p", "frame_id": "f"}]}}
)
HANDSHAKE_LINE = json.dumps({"type": "handshake", "protocol_version": "contextgraph/1.0"})


class _Provider:
    """A provider whose methods do exactly what the test asks, and nothing more."""

    def __init__(
        self,
        query: Any = None,
        verify: Any = None,
        capabilities: Any = None,
    ) -> None:
        self._query = query
        self._verify = verify
        self._capabilities = capabilities

    def info(self) -> dict[str, Any]:
        return INFO

    def capabilities(self) -> dict[str, Any]:
        if self._capabilities is not None:
            return self._capabilities()
        return CAPS

    def query(self, query: dict[str, Any]) -> dict[str, Any]:
        if self._query is not None:
            return self._query(query)
        return {"frames": [], "truncated": False}

    def verify(self, request: dict[str, Any]) -> dict[str, Any]:
        if self._verify is not None:
            return self._verify(request)
        return {
            "verdicts": [{"frame": frame, "status": "valid"} for frame in request["frames"]]
        }


def drive(provider: Any, *lines: str) -> list[dict[str, Any]]:
    """Feed ``lines`` to :func:`run_stdio_provider` and return the replies.

    The runtime owns the real ``sys.stdin``/``sys.stdout``, so redirecting both
    is what lets a unit test read a reply without spawning a child process. EOF
    ends the loop, which is exactly how a host closing the pipe ends a session.
    """
    stdin, stdout, stderr = sys.stdin, sys.stdout, sys.stderr
    sys.stdin = io.StringIO("".join(line + "\n" for line in lines))
    sys.stdout = io.StringIO()
    sys.stderr = io.StringIO()
    try:
        run_stdio_provider(provider)
        written = sys.stdout.getvalue()
    finally:
        sys.stdin, sys.stdout, sys.stderr = stdin, stdout, stderr
    return [json.loads(line) for line in written.splitlines() if line.strip()]


class StdioCrashInputs(unittest.TestCase):
    """Issue #146: two ordinary inputs used to kill the process."""

    def test_non_object_json_lines_are_bad_request_not_a_crash(self) -> None:
        # `json.loads` succeeds for each of these; none has a `.get`.
        for line in ("42", '"x"', "[]", "null", "true"):
            with self.subTest(line=line):
                replies = drive(_Provider(), line)
                self.assertEqual(len(replies), 1, f"{line} must be answered")
                self.assertEqual(replies[0]["type"], "error")
                self.assertEqual(replies[0]["code"], "bad_request")

    def test_the_loop_survives_a_non_object_line_and_answers_the_next_one(self) -> None:
        replies = drive(_Provider(), "42", QUERY_LINE)
        self.assertEqual([r["type"] for r in replies], ["error", "frames"])
        self.assertEqual(replies[1]["id"], "q1")

    def test_query_envelope_without_its_payload_is_bad_request(self) -> None:
        replies = drive(_Provider(), HANDSHAKE_LINE, json.dumps({"type": "query", "id": "q1"}))
        self.assertEqual([r["type"] for r in replies], ["handshake_ack", "error"])
        self.assertEqual(replies[1]["code"], "bad_request")
        # The id is echoed, so the host can free the slot it was holding.
        self.assertEqual(replies[1]["id"], "q1")

    def test_verify_envelope_without_its_payload_is_bad_request(self) -> None:
        replies = drive(_Provider(), json.dumps({"type": "verify"}))
        self.assertEqual(replies[0]["code"], "bad_request")

    def test_a_non_dict_query_payload_is_bad_request(self) -> None:
        replies = drive(_Provider(), json.dumps({"type": "query", "id": "q1", "query": 7}))
        self.assertEqual(replies[0]["code"], "bad_request")

    def test_a_line_that_is_not_json_at_all_is_still_bad_request(self) -> None:
        replies = drive(_Provider(), "{not json")
        self.assertEqual(replies[0]["code"], "bad_request")

    def test_a_blank_line_is_ignored(self) -> None:
        self.assertEqual(drive(_Provider(), "   "), [])


class StdioUnexpectedExceptions(unittest.TestCase):
    """The #156 shape, in Python: an unexpected raise must still answer."""

    def _raise(self, error: BaseException) -> Any:
        def raiser(_payload: dict[str, Any]) -> dict[str, Any]:
            raise error

        return raiser

    def test_unexpected_exception_from_query_is_answered_internal(self) -> None:
        replies = drive(
            _Provider(query=self._raise(TypeError("'NoneType' object is not subscriptable"))),
            QUERY_LINE,
        )
        self.assertEqual(len(replies), 1)
        self.assertEqual(replies[0]["type"], "error")
        self.assertEqual(replies[0]["code"], "internal")
        self.assertEqual(replies[0]["id"], "q1")
        self.assertIn("not subscriptable", replies[0]["message"])

    def test_the_loop_survives_a_crashing_query(self) -> None:
        # The failure this pins: the raise used to escape the loop, so `q2` --
        # already in flight -- was never answered either.
        provider = _Provider(query=self._raise(RuntimeError("index unavailable")))
        replies = drive(
            provider,
            QUERY_LINE,
            json.dumps(
                {
                    "type": "query",
                    "id": "q2",
                    "query": {"goal": "g", "max_frames": 8, "max_tokens": 99},
                }
            ),
        )
        self.assertEqual([r["id"] for r in replies], ["q1", "q2"])
        self.assertTrue(all(r["code"] == "internal" for r in replies))

    def test_provider_error_from_query_keeps_its_own_code(self) -> None:
        replies = drive(
            _Provider(query=self._raise(ProviderError("bad embedding (§E1)", code="bad_request"))),
            QUERY_LINE,
        )
        self.assertEqual(replies[0]["code"], "bad_request")
        self.assertEqual(replies[0]["id"], "q1")

    def test_a_codeless_provider_error_invents_no_code(self) -> None:
        replies = drive(_Provider(query=self._raise(ProviderError("no index loaded"))), QUERY_LINE)
        self.assertEqual(replies[0]["type"], "error")
        self.assertNotIn("code", replies[0])
        self.assertEqual(replies[0]["message"], "no index loaded")

    def test_unexpected_exception_from_verify_is_answered_internal(self) -> None:
        replies = drive(
            _Provider(verify=self._raise(RuntimeError("digest store offline"))), VERIFY_LINE
        )
        self.assertEqual(replies[0]["type"], "error")
        self.assertEqual(replies[0]["code"], "internal")
        self.assertEqual(replies[0]["id"], "v1")

    def test_unexpected_exception_from_capabilities_is_answered_internal(self) -> None:
        def boom() -> dict[str, Any]:
            raise RuntimeError("config not loaded")

        replies = drive(_Provider(capabilities=boom), HANDSHAKE_LINE)
        self.assertEqual(replies[0]["type"], "error")
        self.assertEqual(replies[0]["code"], "internal")

    def test_healthy_paths_are_unchanged(self) -> None:
        replies = drive(_Provider(), HANDSHAKE_LINE, QUERY_LINE, VERIFY_LINE)
        self.assertEqual(
            [r["type"] for r in replies], ["handshake_ack", "frames", "verified"]
        )
        self.assertEqual(replies[1]["id"], "q1")
        self.assertEqual(replies[2]["response"]["verdicts"][0]["status"], "valid")


class HttpMirror(unittest.TestCase):
    """The HTTP adapter answers the same faults the same way."""

    def test_unexpected_exception_from_query_becomes_an_internal_envelope(self) -> None:
        def boom(_payload: dict[str, Any]) -> dict[str, Any]:
            raise RuntimeError("index unavailable")

        stderr, sys.stderr = sys.stderr, io.StringIO()
        try:
            reply = handle_envelope(
                _Provider(query=boom),
                {"type": "query", "id": "q1", "query": {"goal": "g"}},
            )
        finally:
            sys.stderr = stderr
        assert reply is not None
        self.assertEqual(reply["code"], "internal")
        self.assertEqual(reply["id"], "q1")

    def test_unexpected_exception_from_verify_becomes_an_internal_envelope(self) -> None:
        def boom(_request: dict[str, Any]) -> dict[str, Any]:
            raise RuntimeError("digest store offline")

        stderr, sys.stderr = sys.stderr, io.StringIO()
        try:
            status, body = respond_to_body(_Provider(verify=boom), VERIFY_LINE)
        finally:
            sys.stderr = stderr
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body)["code"], "internal")

    def test_missing_payloads_are_bad_request(self) -> None:
        reply = handle_envelope(_Provider(), {"type": "query", "id": "q1"})
        assert reply is not None
        self.assertEqual(reply["code"], "bad_request")
        reply = handle_envelope(_Provider(), {"type": "verify"})
        assert reply is not None
        self.assertEqual(reply["code"], "bad_request")

    def test_a_non_object_body_is_a_400(self) -> None:
        status, body = respond_to_body(_Provider(), "42")
        self.assertEqual(status, 400)
        self.assertEqual(json.loads(body)["code"], "bad_request")

    def test_shutdown_still_ends_the_exchange_without_a_body(self) -> None:
        self.assertEqual(respond_to_body(_Provider(), '{"type":"shutdown"}'), (204, ""))


class ExampleProvidersHonourTheQuery(unittest.TestCase):
    """Issue #147: the example providers must honour `kinds` and `as_of`."""

    @staticmethod
    def _provider() -> Any:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "examples"))
        import example_docs  # noqa: PLC0415

        return example_docs.ExampleDocsProvider()

    @staticmethod
    def _http_provider() -> Any:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "examples"))
        import example_docs_http  # noqa: PLC0415

        return example_docs_http.ExampleDocsHttpProvider()

    def _query(self, provider: Any, **overrides: Any) -> list[dict[str, Any]]:
        query = {"goal": "g", "max_frames": 8, "max_tokens": 4096, **overrides}
        return provider.query(query)["frames"]

    def test_every_declared_kind_is_honoured(self) -> None:
        # The strengthened §Q1 probe narrows to *every* declared kind, not just
        # the first, so each one must come back clean.
        for provider in (self._provider(), self._http_provider()):
            declared = provider.capabilities()["query"]["kinds"]
            self.assertEqual(declared, ["doc", "snippet"])
            for kind in declared:
                with self.subTest(provider=provider.info()["name"], kind=kind):
                    frames = self._query(provider, kinds=[kind])
                    self.assertTrue(frames, "a declared kind should serve something")
                    self.assertTrue(all(f["kind"] == kind for f in frames))

    def test_an_undeclared_kind_returns_nothing_rather_than_off_kind_frames(self) -> None:
        for provider in (self._provider(), self._http_provider()):
            self.assertEqual(self._query(provider, kinds=["fact"]), [])

    def test_an_empty_kinds_list_is_not_a_filter(self) -> None:
        for provider in (self._provider(), self._http_provider()):
            self.assertEqual(len(self._query(provider, kinds=[])), 2)

    def test_an_as_of_pin_excludes_content_not_yet_true(self) -> None:
        pin = "2026-07-01T00:00:00Z"  # the conformance suite's AS_OF_PIN
        for provider in (self._provider(), self._http_provider()):
            frames = self._query(provider, as_of=pin)
            self.assertTrue(frames)
            self.assertTrue(all(f["valid_from"] <= pin for f in frames))

    def test_filtering_is_not_reported_as_truncation(self) -> None:
        # §B2: `truncated` means the provider ran out of budget, not that the
        # host's own narrowing removed something.
        for provider in (self._provider(), self._http_provider()):
            result = provider.query(
                {"goal": "g", "max_frames": 8, "max_tokens": 4096, "kinds": ["doc"]}
            )
            self.assertIs(result["truncated"], False)


if __name__ == "__main__":
    unittest.main()

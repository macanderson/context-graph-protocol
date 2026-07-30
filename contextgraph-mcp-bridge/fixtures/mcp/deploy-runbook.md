# Deploy runbook

Roll out the API in three ordered stages, waiting for the health probe to go
green before advancing:

1. Deploy to `canary` and hold for one full metrics window.
2. Promote to `staging`; run the smoke suite.
3. Promote to `production` behind the progressive rollout flag.

Never skip the canary hold — it is the only stage that sees real traffic before
the blast radius widens.

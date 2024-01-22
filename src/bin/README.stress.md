# stress tool

Before investing in this venture more heavily, we must answer the question:

> Can Veilid sustain throughput rates high enough to satisfy demand?

This tool simulates two peers exchanging data over private routes using app_call.

Each peer maintains a private route, which is advertised in a DHT record. Peers
attempt to send the maximum amount of data possible in each app_call (32k) to
these routes.

If the app_call fails or times out, the peer re-checks the DHT because private
routes do change.

All participants ("peers") in the distrans protocol advertise their private
routes with a similar mechanism.

A "peer address" is "DHT key where a peer announces their private route".

Each "stress peer" attempts to maintain N connections to each counterpart, and
replies to app_calls.

## TODO

- [ ] CLI option for number of concurrent connections
- [ ] CLI option for app_call timeout

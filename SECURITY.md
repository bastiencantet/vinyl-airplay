# Security

This alpha has no independent security audit and no guaranteed update window.
The controller requires a bearer token and an explicit write-request header.
The community launcher binds the controller to loopback; it keeps the token
server-side and only exposes an allowlisted set of routes.

The optional LAN web endpoint is for a trusted private network, without per-user
login or TLS. Do not publish it on the Internet. DNS discovery does not certify
a receiver's identity or trustworthiness.

## Report a vulnerability privately

Use the repository's **Security → Report a vulnerability** flow when available.
If that flow is unavailable, open an issue containing only a request for a
private reporting channel. Do not publish exploit details, credentials, personal
network information, or recordings in a public issue.

Do not send real authentication tokens. Synthetic examples and minimal
reproductions are preferred. Coordinated disclosure is welcome.

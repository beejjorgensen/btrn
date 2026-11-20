# NNTP fixture checklist

- Greeting `200` and `201`; unexpected greeting
- TLS success and TLS/connection failure
- `MODE READER` success and non-fatal rejection
- `CAPABILITIES`
- `AUTHINFO USER`/PASS success and `381`, `480`, `481`, `502`
- `LIST ACTIVE`
- `211` group selection and `411` failure
- XOVER/OVER named fields and unnamed standard fields
- dot-stuffed multiline data
- oversized line and 16 MiB response behavior
- missing terminator and malformed framing
- `HEAD`, `BODY`, and `ARTICLE`
- `423`, `430`, abrupt close, timeout, and reconnect

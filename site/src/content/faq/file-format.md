---
question: "What is Hew's file format?"
order: 7
---

Hew's native format is `.hew` — an open, documented format, not a black box. It's a zip container holding a JSON manifest (document tree, materials, metadata) alongside binary geometry buffers, versioned so files stay compatible across releases. Because it's specified openly, other tools can read and write it without reverse-engineering anything.

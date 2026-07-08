# Security Policy

## Reporting a vulnerability

Report suspected vulnerabilities privately through GitHub's private
vulnerability reporting:
[github.com/hew3d/hew/security/advisories/new](https://github.com/hew3d/hew/security/advisories/new)
(the "Report a vulnerability" button on the repository's **Security**
tab). Please do not discuss an unfixed vulnerability in a public issue,
discussion, or pull request.

Reports are acknowledged within seven days. You will be kept informed as
the report is triaged and fixed, and credited in the advisory unless you
prefer otherwise.

## Scope

The parts of Hew that handle untrusted input are the most valuable
targets for review: the file importers (`.hew`, `.skp` via OpenSKP,
`.dae`, `.gltf`) and anything else that parses bytes a user downloaded
from the internet. Crashes on malformed input are ordinary bugs — file
them as [issues](https://github.com/hew3d/hew/issues) — unless they are
exploitable (memory unsafety, resource exhaustion that survives the
existing caps, sandbox or CSP escapes), in which case report them
privately as above.

Vulnerabilities in [OpenSKP](https://github.com/hew3d/openskp) can be
reported the same way in that repository; if you are not sure where a
`.skp` parsing issue belongs, report it here and it will be routed.

## Supported versions

Hew has not yet cut its first public release. Security fixes land on
`main` and ship in the next release; once releases exist, only the
latest release is supported.

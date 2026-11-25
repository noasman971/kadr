# 0002. GPL-3.0-or-later license

- Status: Accepted
- Date: 2026-11-25

## Context

Kadr exists because popular editors lock features behind subscriptions. The project
must stay free for everyone, forever.

## Options considered

1. **GPL-3.0-or-later**: anyone may use, modify and redistribute Kadr, but a
   redistributed modified version must stay open source under the same license.
2. **MPL-2.0**: only modified files must stay open; easier to embed parts of Kadr in
   closed software.

## Decision

GPL-3.0-or-later, chosen by the project owner. It prevents a closed, paid fork of Kadr
from being distributed.

## Consequences

- Every dependency must be compatible with GPL-3.0 (MIT, Apache-2.0, BSD, LGPL, MPL-2.0
  and GPL are). Each one is recorded in
  [THIRD_PARTY_LICENSES.md](../../THIRD_PARTY_LICENSES.md).
- FFmpeg may be built with GPL components (for example libx264) since Kadr is GPL.
- Kadr's code cannot be reused inside closed-source software.

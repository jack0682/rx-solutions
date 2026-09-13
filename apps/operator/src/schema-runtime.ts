import { z } from 'zod';

// Configure before any application schema is constructed. Strict CSP forbids
// both dynamic parsers and the library's otherwise caught eval capability probe.
z.config({ jitless: true });
export { z };

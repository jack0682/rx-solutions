# Offline MELSEC package authoring example

This input is a **SIMULATION-only example**. It does not contain the first laser machine's actual IP, addresses, program, or safety conditions. The two source files also explicitly state TEST ONLY. Production keys and trust policies are not included.

`template.json` defines the logical controller role and close-request slot. `site.json` binds a loopback endpoint to example M/D addresses. `recipe.json` contains example Linux arm64 package metadata. Perform actual generation using the tools or image from the corresponding release.

```sh
rx-device-package template-digest template.json
rx-device-package assemble template.json site.json recipe.json candidate
rx-device-package request candidate YOUR_KEY_ID signing-request.json
```

These commands only create files. Replace KEY_ID with the identifier actually used by the external signer and verification policy. If the template was modified, also update template_digest in site.json from the template-digest output. `site_config` is a semantic digest for the actual configuration authority to verify; do not use the example value as site evidence.

After receiving an external signature, seal/verify under a separate policy. The tools do not automatically create an external signer or operational trust. Follow the [complete authoring procedure](../../../runtime/rx-device-package/README.md).

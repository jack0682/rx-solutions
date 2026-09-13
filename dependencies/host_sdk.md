# Host SDK binding

The Host journal and gate are implemented in Rust. C++ remains at the ROS, SDK, and device-specific native integration boundary. The existing separation into two repositories and two product images is unchanged.

This implementation decision reuses the previously validated normalization, type, and storage implementations and makes the Host's native dispatch atomic unit consistent. It does not duplicate the platform's business decisions in the Host.

The platform's tools/export_host_sdk.py exports rx-domain / rx-ports / rx-storage / rx-protocol and normative specifications and IDL with content hashes. It excludes rx-application and the P Runtime state writer. Solutions builds this SDK without another repository's working path.

The Host build checks the complete SDK file inventory and every digest. It rejects both modifications to existing files and unregistered sources, such as a newly added automatically executed build.rs. This is a code bundle integrity check, not a substitute for signer trust or site qualification.

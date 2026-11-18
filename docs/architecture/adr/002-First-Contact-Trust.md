# ADR-002 – Trust at first contact

**Status:** Accepted  
**Date:** 2026-11-10

## Context

An automatic first registration without a prior trust anchor can assign neither the intended first client nor the intended device reliably. Encryption alone does not solve this assignment.

## Decision

TOFU remains the default. The risk that a foreign client registers first, or that an attacker impersonates another device at first contact, is accepted. After that, the accepted device key is binding.

Optionally, public client keys can be authorised during setup by an authorised local administrator or an already registered client, and expected public device keys can be deposited in the client. Authorising a key on the device closes the open first registration atomically. Deposited keys are checked in the very first handshake; deviations do not lead to a fallback to TOFU. Private keys are not exchanged.

## Consequences

The default installation needs no additional confirmation steps. The alternative requires a trustworthy path for distributing public keys. Client authorisation on the device and device verification in the client remain separate tasks. SSH key formats and SSH access rights are not introduced by this.

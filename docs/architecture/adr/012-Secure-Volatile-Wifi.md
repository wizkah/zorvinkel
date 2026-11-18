# ADR-012 – Secured volatile Wi-Fi setup

**Status:** Experimental; local tests and native Wi-Fi search checked, system acceptance open  
**Date:** 2026-11-12

## Context

The existing wpa_supplicant/dhcpcd adapter needs a secured connection to the desktop client. Wi-Fi changes should still not be stored persistently. The NetworkManager requirement and the restart requirements of the MVP remain unchanged.

## Decision

The existing security profile gets three optional operations: start scan (5), start temporary Wi-Fi connection (6) and query job state (7). Capability bit 4 marks this extension. Clients offer Wi-Fi setup only when support is announced. The encrypted message contract limits Wi-Fi names to 32 bytes, passwords to 8–63 bytes and scan responses to 64 networks.

Each job gets a random, non-zero 64-bit identifier. Start responses only confirm acceptance; completion is queried separately. Every operation requires an authenticated session and a current registration check. Results are bound to the requesting client key.

A bounded background worker processes at most one Wi-Fi job at a time. BLE processing does not wait for Wi-Fi login or scan completion. A fresh scan uses a registered completion event instead of a fixed wait. Up to 128 jobs are kept in the running process; identical repetitions return the same job. Changed payloads with the same identifier are rejected.

Network changes use the existing persistent operation journal without passwords. Before execution, authorisation is checked again; errors trigger restoration of the previous state. After a process restart, pending changes are reconciled with the actual state. Jobs already journalled are not executed again. Scan results and credentials are not stored persistently.

The desktop client keeps open job identifiers during its runtime and queries them after reconnecting. Write jobs are not repeated automatically after transport errors. The GUI offers Wi-Fi selection, manual entry and correctable errors; device Wi-Fi and restart options remain unavailable in native mode.

## Consequences

The isolated use of the adapter from ADR-011 is replaced by the BLE/GUI integration. No saving of network profiles, no automatic restart of a Wi-Fi change and no confirmation of IP reachability. Wi-Fi login and client reachability remain separate displays.

The client's open job assignment does not survive an app restart. After a restart of wpa_supplicant, the existing configuration again determines the Wi-Fi selection. Simultaneous external management programs remain unsafeguarded. An independent security review and real Wi-Fi switching and recovery tests are outstanding.

# ADR-011 – Volatile wpa_supplicant adapter

**Status:** Experimental; isolated use superseded by ADR-012  
**Date:** 2026-11-11

## Context

The test Pi uses wpa_supplicant and dhcpcd. NetworkManager, hostapd and dnsmasq are missing. Network changes should initially not be stored persistently. The valid architecture still provides NetworkManager as the first network adapter.

## Decision

An isolated Linux adapter implements `NetworkControl` over the local wpa_supplicant control socket. It supports status, scan request, cached scan results, temporary WPA2-Personal profiles, connection set-up and return to the previous connection, including activation states of existing profiles.

Credentials are transferred over the local control channel. The adapter writes no network configuration, sends no `SAVE_CONFIG` and restarts no services. The existing dhcpcd continues to handle IP configuration based on the connection state.

Without an integrated AP/DHCP server, AP commands are rejected. The adapter is not integrated automatically into the productive BLE service.

## Consequences

Temporary settings do not meet the MVP restart options. Profile identifiers apply only to the running wpa_supplicant instance. Own profiles additionally get an ownership marker against accidental deletion after an identifier is reused.

A process abort can leave temporary profiles behind; a restart of wpa_supplicant removes unsaved changes. Other management programs can influence or save the shared runtime state. Simultaneous foreign changes are not safeguarded.

Lasting support for this network stack requires an explicitly commissioned adjustment of the architecture plus persistence, recovery and AP integration.

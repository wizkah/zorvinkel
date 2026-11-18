# ADR-005 – Graphical setup client

**Status:** Accepted for the first desktop implementation  
**Date:** 2026-11-11

## Context

Guided setup must be developable and testable without target hardware. Interface and runtime must not tie the hardware-independent use cases to a GUI toolkit.

## Decision

The client optionally uses egui/eframe 0.33.3 with the native Glow renderer. The `gui` feature and the start parameter `--simulate-gui` open the graphical simulation. CLI simulation and BLE test remain independent start modes.

Toolkit-free setup commands and state images live in the application. A bounded background channel executes at most one job; results update the UI thread. Cancelling before execution only reads the current state. A change that has already been executed is not shown as undone by cancelling.

The simulation uses the existing registration, operation and restart flows with in-memory storage adapters. It accesses neither real Wi-Fi credentials nor the client's radio or network settings. All demonstration data is discarded when the program ends.

QR codes are generated from the current device Wi-Fi data of the simulated device. Drafts in input fields become authoritative only after a successful save.

## Consequences

The GUI dependencies stay in the client. Simulation is permanently marked in the interface; local connection, Wi-Fi login and confirmed IP reachability are shown separately. In the simulation, saved credentials deliberately lead to the manual entry path.

Real security and transport integration, platform key stores, minimum operating-system versions, accessibility and installation remain open. A built macOS window replaces neither a Windows/Linux check nor user acceptance.

API references: [eframe](https://docs.rs/eframe/0.33.3/eframe/), [qrcode](https://docs.rs/qrcode/0.14.1/qrcode/).

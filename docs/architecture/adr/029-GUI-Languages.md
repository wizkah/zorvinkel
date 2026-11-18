# ADR-029 – Languages of the graphical interface

**Status:** Accepted  
**Date:** 2026-11-18

## Context

The interface was German only so far. It should support English and German first; further languages should be able to follow without rework.

## Decision

Every visible GUI text is an entry of the enumeration `Text` in the module `gui::i18n`; an entry contains all language versions. If a version is missing, the client does not build. State holds only identifiers (`Text`, `Notice`, `Action`), never rendered strings, so a language switch changes all displays in the next rendering. Values are inserted through numbered placeholders (`{0}`, `{1}`), so a language can change the order. Decimal separators and units (uptime, memory sizes) follow the language.

At start the preferred language of the operating system applies (crate `sys-locale`); if it is not supported, English applies. A selection field in the header bar switches the language at runtime. The choice is saved in the client's data directory in the file `language` and takes precedence over the system language at the next start; the simulation (`--simulate-gui`) saves nothing.

Technical texts (protocol, logs, command line, error messages in the code) and data supplied by the device (device name, service names and groups, SSIDs) stay untranslated and English or unchanged respectively. Errors are delivered as an enumeration and are turned into text only in the GUI.

## Consequences

A new language needs an entry in `Language` (name, locale identifier, number format) and one column per text entry; the compiler names the missing ones. A test checks that all versions are non-empty and have the same placeholders; it does not check wording and line breaks in the interface. The service and device data of the simulation (e.g. "Photo booth", "Music") are English and are not translated. Right-to-left scripts and fonts for non-Latin languages are not taken into account (egui bundles only Latin characters).

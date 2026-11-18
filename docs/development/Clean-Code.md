# Clean Code

These rules apply to all production and test code. Deviations must have a concrete benefit and be justified.

## Readability

- Code and code-related texts are English throughout: identifiers, comments, API documentation, test descriptions, and log and technical error messages.
- Names describe purpose and meaning. Avoid unusual abbreviations.
- Functions stay small and fulfil one clearly named task.
- Use a consistent level of abstraction within a function. Express domain flows and technical details separately.
- Comments explain reasons and constraints. Do not retell the code.
- Remove unused code and outdated comments.

## Responsibilities and dependencies

- Separation of Concerns: separate domain logic, flow control, presentation and external access.
- Single Responsibility: each unit has one coherent responsibility and one clear reason to change.
- Open/Closed: make foreseeable variants extensible in a targeted way. Do not build extension points in advance.
- Liskov Substitution: implementations honour the guaranteed contracts of their abstractions.
- Interface Segregation: interfaces contain only the capabilities their users need.
- Dependency Inversion: domain rules do not depend on concrete infrastructure implementations.
- Make dependencies and mutable state explicit. Avoid hidden global dependencies.

## Flow control and operations

- Apply IOSP where it improves readability and testability: integration coordinates operations; operations do the actual processing.
- Encapsulate domain decisions in operations. Integrations concentrate on how they work together.
- Express calculations as pure functions where possible. Keep input/output and other side effects clearly separated.
- Do not introduce additional layers or micro-functions merely to satisfy a principle formally.

## Data and errors

- Express valid states through suitable types and clear contracts.
- Validate data at system boundaries. Rely on defined contracts internally.
- Keep data immutable where possible; assign changes and resource ownership unambiguously.
- Handle errors explicitly or pass them on with their context. Do not swallow errors and do not fake success.
- Release resources also on errors and aborts.
- Do not put secrets in code or write them to logs.

## Simplicity

- Choose the simplest solution that meets the current need.
- Do not duplicate knowledge and domain rules. Merge similar code only when it shares a meaning.
- Introduce abstractions only for a concrete need.
- Unify formatting and automatable rules through shared tools.

## Tests

- Test observable behaviour and contracts; do not couple tests to internal implementation details.
- Test domain logic in isolation and secure the interplay of external components with integration tests.
- Consider relevant edge cases, errors and aborts.
- Keep tests reproducible. Make time, randomness and external systems controllable where needed.

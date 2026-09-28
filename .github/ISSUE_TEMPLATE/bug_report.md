name: Bug report
description: Something works differently than documented or expected
labels: ["bug"]
body:
  - type: textarea
    id: what-happened
    attributes:
      label: What happened?
      description: What you did, what you expected, what you got instead.
      placeholder: |
        qv <alias> <args> → ...
        expected: ...
        got: ...
    validations:
      required: true
  - type: input
    id: version
    attributes:
      label: Quiver version
      description: Output of the release tag, or `git rev-parse HEAD` for a source build.
      placeholder: v0.1.0
    validations:
      required: true
  - type: input
    id: platform
    attributes:
      label: OS / Wox version
      placeholder: Windows 11 / Wox 2.4.2
    validations:
      required: true
  - type: textarea
    id: catalog-entry
    attributes:
      label: Catalog entry (if relevant)
      description: The alias's JSON — redact anything private.
      render: json
  - type: textarea
    id: logs
    attributes:
      label: Logs
      description: |
        Reproduce with `QUIVER_LOG=debug` and paste stderr (Wox's log file, or run the binary standalone).

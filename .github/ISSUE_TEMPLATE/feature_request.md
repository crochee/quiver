name: Feature request
description: A new alias capability, platform, or catalog field
labels: ["enhancement"]
body:
  - type: textarea
    id: problem
    attributes:
      label: What are you trying to do?
      description: The launcher habit you want, not the implementation you imagine.
      placeholder: I type `qv <alias> ...` and I want ...
    validations:
      required: true
  - type: textarea
    id: today
    attributes:
      label: What gets in the way today?
    validations:
      required: true
  - type: dropdown
    id: area
    attributes:
      label: Which surface would this change?
      options:
        - ShellCommands.json catalog fields (data only)
        - Placeholders ({query} / $@ / $N)
        - capture / silent semantics
        - New interpreter / platform support
        - Build / packaging

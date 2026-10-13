# Coding Style Preferences

- Prioritizes small binary size, low idle memory usage, and near-zero CPU when idle. Confidence: 0.8
- Prefers event-driven designs over polling (e.g. reacting to change signals rather than polling loops). Confidence: 0.8
- Wants errors handled gracefully without panics, with logging to stderr via a minimal, dependency-free logger. Confidence: 0.75
- Cares about not leaking resources, e.g. memory when processing images. Confidence: 0.6

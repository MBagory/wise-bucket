# Principles

1. **The LLM reasons; tools measure.** Numbers come from computations Wise Bucket runs on your recordings, never from a model's memory. A hypothesis is only as good as the evidence it cites.
2. **Evidence over plausibility.** Every metric, observation and interpretation points to its source: which recording (and which exact version of the file), which stream, which time window, which computation.
3. **Nothing is invented.** Unknown context is asked for, not guessed. What the engineer said is recorded as *stated by the user*, not as a measured fact.
4. **Clocks and uncertainty are explicit.** Timestamps keep nanosecond precision. Comparing signals from different clocks requires a known mapping; otherwise the answer is "cannot be resolved", not a number.
5. **Local first.** Recordings, the database and the analysis run on your machine. Only what your harness sends to its model leaves it (see [Trust and privacy](trust-and-privacy.md)).
6. **Your harness owns the model and the budget.** Wise Bucket works with the AI tool you already use. It never calls a model and never spends your tokens by itself.
7. **Boundaries are set by you.** Wise Bucket reads only the folders you declare as *data roots*. There is deliberately no tool that lets the AI add a root.
8. **Built one milestone at a time.** Each milestone is small, tested and documented before the next one starts.

The folder `{{DIR}}` holds public sample recordings of the format `{{FORMAT}}`. Wise Bucket, an MCP server, has them in its `demo` root.

Investigate them twice, in this order:

1. **On your own**, from the raw files in this folder, with whatever tools you choose. Do not use the Wise Bucket tools, the `wisebucket` command or Wise Bucket's store during this step.
2. **Only through the Wise Bucket tools.** If `server_info` shows recordings still `pending`, they are being extracted: call it again.

Each time, find what the machine did, its configuration (hardware, firmware, parameters), the errors and warnings it logged, anomalies, data gaps, durations and rates.

Then compare, as a Markdown report:

- **Both:** found both ways; say whether the values match.
- **Disagree:** the same fact with different values; give both.
- **Wise Bucket only:** what Wise Bucket gave that your own investigation did not, or only at a much higher cost.
- **Own investigation only:** what the files contain that Wise Bucket did not give, or gave wrongly.

For each item give the value and the evidence (code or SQL query). End with what Wise Bucket should change, ranked by how much it would help an agent investigating such recordings.

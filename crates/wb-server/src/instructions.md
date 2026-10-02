Wise Bucket helps an engineer investigate robot behaviour iteratively: questions, hypotheses, recordings (ROS 2 bags, flight logs, CAN captures), metrics and interpretations, kept across conversations.

This is milestone M0 (foundation). Available now: `server_info`, which reports the installation state and the data roots (the only folders Wise Bucket may read).

Guidelines:
- Call `server_info` when the user asks about Wise Bucket's setup, or before relying on data roots.
- If a result contains an `error` object, tell the user its `message` and `fix`, and give the `docs_ref` link. Do not guess workarounds.
- Data roots are declared by the engineer with the `wisebucket roots add` command. Never try to add or bypass roots.

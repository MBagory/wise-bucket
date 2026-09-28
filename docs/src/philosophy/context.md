# Context: why Wise Bucket

## The problem

Robotics R&D is iterative. A typical investigation looks like this:

```text
question → hypotheses → physical test → recording → analysis → updated hypotheses → next test …
```

General-purpose AI assistants are good at the reasoning part: they know control theory, sensor failure modes and communication buses. But on their own they:

- **forget** everything between conversations: your robot, your sensors, what you tested last week;
- **guess** instead of measuring, because large recordings do not fit in a chat;
- **lose provenance:** a number in a chat has no link to the file, topic, time window and computation it came from;
- **mix up clocks:** a ROS bag, a flight-controller log and a CAN capture from the same test each have their own time base.

## What Wise Bucket adds

Wise Bucket is the **memory and the measuring instrument** your AI harness is missing:

| Need | Wise Bucket |
| --- | --- |
| Remember the robot world | A knowledge base of robots, sensors, buses and software, with provenance *(M1)* |
| Structure the investigation | Questions, hypotheses with testable predictions, data requests *(M2)* |
| Read the real data | Native readers for ROS 2 bags and other robotics formats *(M3)* |
| Trustworthy numbers | Metrics computed by Wise Bucket, each traceable to its source *(M4)* |
| Find things again | Hybrid semantic + keyword search over everything attached to your logs *(M1–M4)* |

## What Wise Bucket is not

- **Not an LLM.** It never calls a model. Your harness owns the model, your API keys and your budget.
- **Not a cloud service.** Everything runs on your machine; recordings never leave it.
- **Not a robot controller.** It reads recordings; it never talks to a live robot.

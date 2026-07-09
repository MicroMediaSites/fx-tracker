# Embedded AI Agent Architecture

Generic pattern for embedding an agentic AI workflow directly into application UI.

Paste into [mermaid.live](https://mermaid.live) to render.

```mermaid
flowchart TB
    subgraph UI["Application UI"]
        direction LR
        P1["Page A"]
        P2["Page B"]
        P3["Page C"]
    end

    UI -->|"each page exposes<br/>a context provider"| AGENT["Embedded Agent<br/>(overlay / panel / sidebar)"]
    AGENT -->|"user prompt"| CLASSIFY

    CLASSIFY["Lightweight Classifier<br/>(fast, cheap model — e.g. GPT-4o-mini)<br/>Categorizes the query"]

    CLASSIFY -->|"classification"| CTX

    subgraph CTX["Selective Context Loading"]
        direction LR
        DECIDE{"category"}
        DECIDE -->|"A"| CA["Minimal context"]
        DECIDE -->|"B"| CB["Domain-specific<br/>context"]
        DECIDE -->|"C"| CC["Heavy context<br/>(data, metrics)"]
    end

    subgraph HISTORY["Chat History Management"]
        direction TB
        STORE[("Persistent Store<br/>(DB / local)")]
        STORE --> BUILD["Build API history"]
        BUILD --> COMP_CHK{"history<br/>exceeds<br/>threshold?"}
        COMP_CHK -->|"no"| HIST["Bounded history"]
        COMP_CHK -->|"yes"| COMPACT["Rolling Compaction<br/>(cheap model summarizes<br/>oldest messages into<br/>a running summary)"]
        COMPACT -->|"replace oldest msgs<br/>with compacted summary"| STORE
        COMPACT --> HIST
        HIST -.->|"compacted summary<br/>prepended to future calls<br/>(not re-compacted each time)"| HIST
    end

    subgraph ASSEMBLE["Request Assembly"]
        direction TB
        SYS["System Prompt<br/>+ cache_control: ephemeral"]
        SYS --- PAYLOAD
        HIST_IN["Compacted History"] --- PAYLOAD
        CTX_IN["Selected Context"] --- PAYLOAD
        PROMPT_IN["Current Prompt"] --- PAYLOAD
        PAYLOAD["Assembled Payload"]
    end

    CA & CB & CC --> CTX_IN
    HIST --> HIST_IN

    PAYLOAD --> ROUTE

    subgraph ROUTE_BOX["Model Routing"]
        direction LR
        ROUTE{"task<br/>complexity?"}
        ROUTE -->|"capable"| BIG["Large Model<br/>(Opus-class)<br/>Tools enabled"]
        ROUTE -->|"fast"| SMALL["Small Model<br/>(Haiku-class)<br/>Tools enabled"]
    end

    BIG & SMALL --> STREAM

    subgraph STREAM_BOX["Streaming Response"]
        STREAM["SSE Stream"]
        STREAM -->|"text delta"| RENDER["Stream to UI"]
        STREAM -->|"tool call"| TOOL["Execute Tool<br/>(APIs, DB, etc.)"]
        STREAM -->|"complete"| SAVE["Save to<br/>history store"]
        TOOL -->|"tool result<br/>→ continue generation"| STREAM
    end

    SAVE --> STORE

    style CLASSIFY fill:#4a9eff,color:#fff
    style BIG fill:#8b5cf6,color:#fff
    style SMALL fill:#22c55e,color:#fff
    style COMPACT fill:#f59e0b,color:#fff
    style STORE fill:#06b6d4,color:#fff
    style AGENT fill:#10b981,color:#fff
    style PAYLOAD fill:#6366f1,color:#fff
    style SYS fill:#ec4899,color:#fff
```

## Token Efficiency Strategies

| Strategy | How | Savings |
|---|---|---|
| **Classification** | Cheap model categorizes query so only relevant context is loaded | Avoids sending full app state on every request |
| **Rolling Compaction** | Cheap model summarizes oldest messages into a running summary when history exceeds a threshold — summary is prepended to future calls, not recomputed each time | Bounds history growth — O(1) instead of O(n) |
| **Prompt Caching** | `cache_control: ephemeral` on system prompt | Cache hit skips re-processing system prompt tokens (often thousands) |
| **Model Routing** | Route to smaller model for simpler tasks | Direct cost reduction per request |

## Key Design Decisions

1. **Classifier is separate from the main LLM** — uses a fast, cheap model so classification cost is negligible
2. **Context is pulled, not pushed** — pages expose a provider; the agent only calls it after classification tells it what's needed
3. **Compaction is rolling, not per-request** — maintains a bounded summary over time rather than compressing on every call
4. **Prompt cache lives on the system prompt** — the part that rarely changes gets cached; context and history (which change) don't
5. **Tool loop is recursive** — tool results feed back into the stream, enabling multi-step agentic workflows

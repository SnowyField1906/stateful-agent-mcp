If you see this text, it means you are using `agent-mcp` and working on its own source to fix or improve something.
Try the MCP and improve yourself (you may or may not be the updated version with changes from the last session, the user will explicitly mention this).
While working with this MCP and on this MCP, also take notice of these side tasks that match its philosophy:

A. TOKEN OPTIMIZATION: Optimizing the MCP's instruction/description literature (dense information, clean literature, consistent texture, no verbosity, etc.) not only optimizes token usage, but it will also unexpectedly improve other aspects for the AI (avoiding hallucination, distraction, confusion, etc.):

1. Only 20% of the tokens are needed to convey 80% of the information. And information, as some meaning, can be derived:
   - Implicitly: From Patterns, Constraints, Connections, Heuristics/Intuition, etc.
   - Explicitly: From Tool Literature (instructions and descriptions), Tool Structure (design, naming, purposes, DTOs, params, etc.), Reinforced Experiences (the DTOs and param namings in requests and responses are also re-presented or duplicated in the context window).
2. To respect (1):
   - Consistency is the top priority. Clear, precise language, clear structure, and clear purpose are all encouraged.
   - Only information that is truly important or orthogonal is presented. For other information, it should be "shown" by various means — both implicitly and explicitly.
   - Proactive forms of habits for AI clients (like some DTOs in responses being the same as params so the AI doesn't need to learn them, like the tool itself giving short, valuable hints with flexibility and robust handles to prevent trial-and-error or long descriptions, like instead of repeatedly asking it to use clear language and write down important things only, you can do so to make the vibe for AI clients, etc.).
3. Further than that, the effectiveness of each feature and the frictions during usage also need to be evaluated carefully while using it on your own.
   - "I'm also a client of this MCP, is there anything I expected it to be/have so that this work could have been done faster?"
   - "Was there anything that made me confused and I had to guess on my own? Or was there anything that made me easily deviate from the optimal workflow you expect?"
   - "Can the instructions and descriptions this MCP gives to me be better?"
   - "Is there any unnecessary or repeated information in the DTOs or in the instructions that distracts from and dilutes my extreme optimization over information?"

B. "LLM-ERGONOMIC" WORKFLOW:

1. Aside from the question "can the AI client quickly understand what it should do?", you also need to ask "can the AI client quickly know what it can do or the MCP's capabilities?"
2. Also ask "when I did everything to optimize tokens, is it worth it and balanced with work effectiveness?"
3. "How to make the optimal workflow become the AI client's instincts so that it can work smoothly without any confusion and be okay with token-limit run-out interruptions or continuing an interrupted turn in other fresh sessions?"

C. "RELAY RACE" WORKFLOW:

1. I designed a set of philosophies called "LLM-ergonomic workflow"; aside from making the AI client operate it smoothly, it is also very important to make the AI able to quickly join the race in a fresh session without facing significant disadvantages or confusion.
2. The reason tokens needed to be optimized is because of Claude's strict token-usage limits. That's why there is a set of file diffs, quick insights, and small short-term tasks to keep picking up like a relay race.

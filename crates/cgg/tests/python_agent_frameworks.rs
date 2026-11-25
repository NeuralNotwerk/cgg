//! Python AI-agent and tool-calling SDK rules added in 0.14.0 (agent
//! research 2026-11-25). One fixture per SDK, the research's own, asserting
//! the entry count per rule and, for tools, that the entry is routed by the
//! tool's name.

use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn run(files: &[(&str, &str)]) -> (String, String) {
    let tmp = TempDir::new().unwrap();
    for (rel, body) in files {
        let p = tmp.path().join(rel);
        fs::create_dir_all(p.parent().unwrap_or(Path::new("."))).unwrap();
        fs::write(p, body).unwrap();
    }
    let out = tmp.path().join("g.mmd");
    let assert = Command::cargo_bin("cgg")
        .expect("cgg binary")
        .args(["--jobs", "1", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

fn assert_entries(err: &str, id: &str, kind: &str, n: usize) {
    let want = if n == 1 {
        format!(" {id} ({kind}, 1 entry)")
    } else {
        format!(" {id} ({kind}, {n} entries)")
    };
    assert!(err.contains(&want), "expected `{want}` in:\n{err}");
}

#[test]
fn mcp_fastmcp_tools_resources_prompts_and_add_tool() {
    let (g, err) = run(&[
        (
            "server.py",
            r###"from mcp.server.fastmcp import FastMCP, Context

mcp = FastMCP("Demo")


@mcp.tool()
def add(a: int, b: int) -> int:
    """Shape: @mcp.tool()."""
    return a + b


@mcp.tool
def bare(a: int) -> int:
    """Shape: @mcp.tool bare (FastMCP 2)."""
    return a


@mcp.resource("greeting://{name}")
def get_greeting(name: str) -> str:
    """Shape: @mcp.resource(uri)."""
    return name


@mcp.prompt()
def review_code(code: str) -> str:
    """Shape: @mcp.prompt()."""
    return code


def added_later(x: int) -> int:
    """Shape: mcp.add_tool(fn)."""
    return x


mcp.add_tool(added_later)


def never_used():
    """Control."""


if __name__ == "__main__":
    mcp.run()
"###,
        ),
        (
            "server2.py",
            r###"from fastmcp import FastMCP

app = FastMCP("v2")


@app.tool(name="mult", tags={"math"})
def multiply(a: float, b: float) -> float:
    """Shape: FastMCP 2 @app.tool(name=...)."""
    return a * b


@app.resource("data://config")
def config() -> dict:
    return {}
"###,
        ),
    ]);
    assert_entries(&err, "mcp-python", "tool", 7);
    assert!(g.contains("tool::mcp-python::mult"), "{g}");
    assert!(g.contains("tool::mcp-python::greeting://{name}"), "{g}");
}

#[test]
fn mcp_low_level_handlers_and_v2_server_slots() {
    let (g, err) = run(&[
        (
            "server.py",
            r###"import mcp.types as types
from mcp.server.lowlevel import Server

server = Server("low")


@server.list_tools()
async def handle_list_tools() -> list[types.Tool]:
    """Shape: @server.list_tools()."""
    return []


@server.call_tool()
async def handle_call_tool(name: str, arguments: dict) -> list[types.TextContent]:
    """Shape: @server.call_tool() - dispatches by name to helpers."""
    if name == "fetch":
        return await fetch_impl(arguments)
    return []


async def fetch_impl(arguments):
    return []


@server.list_resources()
async def handle_list_resources():
    return []


@server.read_resource()
async def handle_read_resource(uri):
    return ""


def never_used():
    """Control."""
"###,
        ),
        (
            "server_v2.py",
            r###"from mcp.server.lowlevel import Server
from mcp.server import MCPServer


async def my_list_tools(ctx, params):
    """Shape: mcp 2.x Server(on_list_tools=fn)."""
    return []


async def my_call_tool(ctx, params):
    """Shape: mcp 2.x Server(on_call_tool=fn)."""
    return []


def build():
    return Server("low2", on_list_tools=my_list_tools, on_call_tool=my_call_tool)


app = MCPServer("Demo")


@app.tool()
def v2_tool(a: int) -> int:
    return a
"###,
        ),
    ]);
    assert_entries(&err, "mcp-python", "tool", 7);
    let _ = g;
}

#[test]
fn dspy_forward_and_react_tools() {
    let (g, err) = run(&[(
        "app.py",
        r###"import dspy


class QA(dspy.Signature):
    """Answer questions."""
    question: str = dspy.InputField()
    answer: str = dspy.OutputField()


class RAG(dspy.Module):
    def __init__(self):
        super().__init__()
        self.gen = dspy.ChainOfThought(QA)

    def forward(self, question):
        return self.gen(question=question)


def ds_search(query: str) -> list[str]:
    """tool passed to ReAct"""
    return []


def ds_calc(expr: str) -> float:
    """tool wrapped in dspy.Tool"""
    return 0.0


def ds_metric(example, pred, trace=None):
    return example.answer == pred.answer


react = dspy.ReAct("question -> answer", tools=[ds_search, dspy.Tool(ds_calc)])
optimizer = dspy.MIPROv2(metric=ds_metric)
compiled = optimizer.compile(RAG(), trainset=[])
"###,
    )]);
    assert_entries(&err, "dspy", "lifecycle", 1);
    assert_entries(&err, "dspy-tools", "tool", 2);
    let _ = g;
}

#[test]
fn langchain_tools_every_registration_form() {
    let (g, err) = run(&[(
        "tools.py",
        r###"from langchain_core.tools import tool, StructuredTool, BaseTool, Tool
from langchain.agents import create_agent
from langchain_openai import ChatOpenAI


@tool
def lc_search(query: str) -> str:
    """Search the web (bare @tool)."""
    return "x"


@tool("weather_lookup", return_direct=True)
def lc_weather(city: str) -> str:
    """Weather (@tool with args)."""
    return "sunny"


def lc_multiply(a: int, b: int) -> int:
    """Multiply (StructuredTool.from_function)."""
    return a * b


multiply_tool = StructuredTool.from_function(func=lc_multiply, name="multiply")


def lc_legacy(q: str) -> str:
    """Legacy Tool(func=...)."""
    return q


legacy_tool = Tool(name="legacy", func=lc_legacy, description="legacy")


class CalculatorTool(BaseTool):
    name: str = "calculator"
    description: str = "subclass BaseTool"

    def _run(self, expression: str) -> str:
        return str(len(expression))

    async def _arun(self, expression: str) -> str:
        return self._run(expression)


def lc_plain_fn(x: str) -> str:
    """Plain function passed to bind_tools."""
    return x


def lc_agent_fn(x: str) -> str:
    """Plain function passed to create_agent(tools=[...])."""
    return x


llm = ChatOpenAI(model="gpt-4o")
llm_with_tools = llm.bind_tools([lc_search, lc_plain_fn, multiply_tool])
agent = create_agent(llm, tools=[lc_agent_fn, CalculatorTool()])
"###,
    )]);
    assert_entries(&err, "langchain-python", "tool", 8);
    assert!(g.contains("tool::langchain-python::weather_lookup"), "{g}");
}

#[test]
fn langchain_middleware_and_callbacks() {
    let (g, err) = run(&[(
        "mw.py",
        r###"from langchain.agents import create_agent
from langchain.agents.middleware import AgentMiddleware, before_model, wrap_tool_call, dynamic_prompt
from langchain_core.callbacks import BaseCallbackHandler


class Guard(AgentMiddleware):
    def before_model(self, state, runtime):
        return None

    def wrap_tool_call(self, request, handler):
        return handler(request)


@before_model
def mw_trim(state, runtime):
    return None


@dynamic_prompt
def mw_prompt(request):
    return "sys"


class Cb(BaseCallbackHandler):
    def on_tool_start(self, serialized, input_str, **kw):
        pass
"###,
    )]);
    assert_entries(&err, "langchain-python-middleware", "lifecycle", 5);
    let _ = g;
}

#[test]
fn langgraph_nodes_routers_and_tools() {
    let (g, err) = run(&[(
        "graph.py",
        r###"from typing import TypedDict
from langgraph.graph import StateGraph, START, END
from langgraph.prebuilt import ToolNode, create_react_agent, tools_condition
from langgraph.func import entrypoint, task
from langgraph.checkpoint.memory import InMemorySaver
from langchain_core.tools import tool


class State(TypedDict):
    n: int


def lg_node_a(state: State) -> dict:
    return {"n": state["n"] + 1}


def lg_node_b(state: State) -> dict:
    return {"n": state["n"] * 2}


def lg_named_node(state: State) -> dict:
    return {"n": 0}


def lg_router(state: State) -> str:
    return "b" if state["n"] > 1 else END


def lg_entry(state: State) -> dict:
    return state


def lg_finish(state: State) -> dict:
    return state


@tool
def lg_tool(q: str) -> str:
    """tool used by ToolNode"""
    return q


def lg_tool_plain(q: str) -> str:
    """plain fn tool used by ToolNode"""
    return q


def lg_react_tool(q: str) -> str:
    """plain fn tool used by create_react_agent"""
    return q


def lg_prompt(state):
    return [{"role": "system", "content": "hi"}]


builder = StateGraph(State)
builder.add_node(lg_node_a)                 # name inferred from function
builder.add_node("b", lg_node_b)            # explicit name
builder.add_node("named", action=lg_named_node)
builder.add_node("tools", ToolNode([lg_tool, lg_tool_plain]))
builder.add_node("entry", lg_entry)
builder.add_node("finish", lg_finish)
builder.set_entry_point("entry")
builder.set_finish_point("finish")
builder.add_edge(START, "lg_node_a")
builder.add_conditional_edges("lg_node_a", lg_router, {"b": "b", END: END})
builder.add_conditional_edges("b", tools_condition)
graph = builder.compile(checkpointer=InMemorySaver())

react = create_react_agent("openai:gpt-4o", tools=[lg_react_tool], prompt=lg_prompt)


@task
def lg_task_step(x: int) -> int:
    return x + 1


@entrypoint(checkpointer=InMemorySaver())
def lg_workflow(inputs: dict) -> int:
    fut = lg_task_step(inputs["x"])
    return fut.result()
"###,
    )]);
    assert_entries(&err, "langgraph-python", "lifecycle", 8);
    assert_entries(&err, "langgraph-python-tools", "tool", 2);
    let _ = g;
}

#[test]
fn llamaindex_steps_and_function_tools() {
    let (g, err) = run(&[(
        "app.py",
        r###"from llama_index.core.workflow import Workflow, step, StartEvent, StopEvent, Event, Context
from llama_index.core.tools import FunctionTool, QueryEngineTool, BaseTool
from llama_index.core.agent.workflow import FunctionAgent, ReActAgent, AgentWorkflow
from llama_index.llms.openai import OpenAI


class MidEvent(Event):
    text: str


class MyFlow(Workflow):
    @step
    async def li_start(self, ev: StartEvent) -> MidEvent:
        return MidEvent(text=ev.topic)

    @step(num_workers=4)
    async def li_finish(self, ctx: Context, ev: MidEvent) -> StopEvent:
        return StopEvent(result=ev.text)


@step(workflow=MyFlow)
async def li_free_step(ev: MidEvent) -> StopEvent:
    return StopEvent(result="x")


def li_multiply(a: int, b: int) -> int:
    """FunctionTool.from_defaults"""
    return a * b


def li_add(a: int, b: int) -> int:
    """bare function in tools=[...]"""
    return a + b


async def li_search(q: str) -> str:
    """async function as FunctionTool(async_fn=)"""
    return q


multiply_tool = FunctionTool.from_defaults(fn=li_multiply)
search_tool = FunctionTool.from_defaults(async_fn=li_search)
agent = FunctionAgent(tools=[multiply_tool, li_add], llm=OpenAI(model="gpt-4o"))


def build():
    return ReActAgent(tools=[li_add], llm=OpenAI())
"###,
    )]);
    assert_entries(&err, "llamaindex-workflows", "lifecycle", 3);
    assert_entries(&err, "llamaindex-tools", "tool", 3);
    let _ = g;
}

#[test]
fn crewai_tools_project_and_flow() {
    let (g, err) = run(&[(
        "crew.py",
        r###"from crewai import Agent, Crew, Process, Task
from crewai.project import CrewBase, agent, crew, task, before_kickoff, after_kickoff
from crewai.tools import BaseTool, tool
from crewai.flow.flow import Flow, start, listen, router, or_, and_
from pydantic import BaseModel


@tool("Search the web")
def search_tool(query: str) -> str:
    """Shape: @tool("name") decorator."""
    return query


@tool
def bare_tool(query: str) -> str:
    """Shape: bare @tool."""
    return query


class MyCustomTool(BaseTool):
    name: str = "custom"
    description: str = "Shape: BaseTool subclass; runtime calls _run."

    def _run(self, argument: str) -> str:
        return argument

    async def _arun(self, argument: str) -> str:
        return argument


@CrewBase
class ResearchCrew:
    agents_config = "config/agents.yaml"
    tasks_config = "config/tasks.yaml"

    @before_kickoff
    def prepare_inputs(self, inputs):
        return inputs

    @after_kickoff
    def process_output(self, output):
        return output

    @agent
    def researcher(self) -> Agent:
        return Agent(config=self.agents_config["researcher"], tools=[search_tool, MyCustomTool()])

    @task
    def research_task(self) -> Task:
        return Task(config=self.tasks_config["research_task"], callback=task_done, guardrail=check_output)

    @crew
    def crew(self) -> Crew:
        return Crew(agents=self.agents, tasks=self.tasks, process=Process.sequential, step_callback=on_step)


def task_done(output):
    """Shape: Task(callback=fn)."""


def check_output(result):
    """Shape: Task(guardrail=fn)."""
    return (True, result)


def on_step(step):
    """Shape: Crew(step_callback=fn)."""


class State(BaseModel):
    n: int = 0


class MyFlow(Flow[State]):
    @start()
    def begin(self):
        return 1

    @listen(begin)
    def second(self, x):
        return x

    @router(second)
    def route(self):
        return "ok"

    @listen("ok")
    def on_ok(self):
        pass

    @listen(or_(begin, second))
    def either(self):
        pass


def never_used():
    """Control."""


def run():
    ResearchCrew().crew().kickoff(inputs={})
    MyFlow().kickoff()
"###,
    )]);
    assert_entries(&err, "crewai-tools", "tool", 4);
    assert_entries(&err, "crewai-project", "lifecycle", 8);
    assert_entries(&err, "crewai-flow", "lifecycle", 5);
    assert!(g.contains("tool::crewai-tools::search_tool"), "{g}");
    assert!(g.contains("tool::crewai-tools::MyCustomTool"), "{g}");
}

#[test]
fn strands_tools_and_hooks() {
    let (g, err) = run(&[
        (
            "agent.py",
            r###"from strands import Agent, tool
from strands.hooks import HookProvider, HookRegistry
from strands.hooks import BeforeToolCallEvent, AfterInvocationEvent, BeforeModelCallEvent
from strands.multiagent import GraphBuilder, Swarm
from strands.types.tools import AgentTool
import tools.word_count as word_count_mod


def lookup(city):
    return city.upper()


@tool
def weather(city: str) -> str:
    """Get the weather for a city."""
    return lookup(city)


@tool(name="calc", description="Evaluate an expression")
def calculator(expr: str) -> str:
    return expr


class Toolbox:
    @tool
    def search(self, q: str) -> str:
        """Search."""
        return q


class LoggingHooks(HookProvider):
    def register_hooks(self, registry: HookRegistry, **kwargs) -> None:
        registry.add_callback(BeforeToolCallEvent, self.log_start)
        registry.add_callback(AfterInvocationEvent, self.log_end)

    def log_start(self, event: BeforeToolCallEvent) -> None:
        print(event)

    def log_end(self, event: AfterInvocationEvent) -> None:
        print(event)


def on_model_call(event: BeforeModelCallEvent) -> None:
    print(event)


def stream_printer(**kwargs):
    print(kwargs)


class Shout(AgentTool):
    @property
    def tool_name(self):
        return "shout"

    @property
    def tool_spec(self):
        return {"name": "shout"}

    async def stream(self, tool_use, invocation_state, **kwargs):
        yield tool_use


def only_if_research_done(state) -> bool:
    return True


def build():
    box = Toolbox()
    agent = Agent(
        tools=[weather, calculator, box.search, word_count_mod, Shout(),
               "./tools/file_tool.py"],
        hooks=[LoggingHooks()],
        callback_handler=stream_printer,
    )
    agent.hooks.add_callback(BeforeModelCallEvent, on_model_call)
    return agent


def build_graph():
    researcher = Agent(name="researcher")
    writer = Agent(name="writer")
    b = GraphBuilder()
    b.add_node(researcher, "research")
    b.add_node(writer, "write")
    b.add_edge("research", "write", condition=only_if_research_done)
    return b.build(), Swarm([researcher, writer])


if __name__ == "__main__":
    build()("hi")
    build_graph()


from strands.plugins import Plugin, hook


class AuditPlugin(Plugin):
    name = "audit"

    @hook
    def on_tool(self, event: BeforeToolCallEvent) -> None:
        print(event)

    @tool
    def audit_log(self, line: str) -> str:
        return line

    def init_agent(self, agent) -> None:
        print(agent)
"###,
        ),
        (
            "tools/file_tool.py",
            r###"TOOL_SPEC = {"name": "file_tool", "description": "x", "inputSchema": {"json": {}}}


def file_tool(tool, **kwargs):
    return {"toolUseId": tool["toolUseId"], "status": "success", "content": []}
"###,
        ),
        (
            "tools/word_count.py",
            r###"from strands.types.tools import ToolResult, ToolUse

TOOL_SPEC = {
    "name": "word_count",
    "description": "Count words",
    "inputSchema": {"json": {"type": "object"}},
}


def _split(text):
    return text.split()


def word_count(tool: ToolUse, **kwargs) -> ToolResult:
    n = len(_split(tool["input"]["text"]))
    return {"toolUseId": tool["toolUseId"], "status": "success",
            "content": [{"text": str(n)}]}
"###,
        ),
    ]);
    assert_entries(&err, "strands-python", "tool", 6);
    assert_entries(&err, "strands-python-hooks", "lifecycle", 7);
    assert!(g.contains("tool::strands-python::calc"), "{g}");
}

#[test]
fn pydantic_ai_tools_and_hooks() {
    let (g, err) = run(&[(
        "app.py",
        r###"from dataclasses import dataclass
from pydantic_ai import Agent, RunContext, Tool, ModelRetry
from pydantic_ai.toolsets import FunctionToolset


@dataclass
class Deps:
    db: str


def lookup_weather(city: str) -> str:
    """Shape: function passed as a value in tools=[...]."""
    return f"sunny in {city}"


def tool_via_wrapper(ctx: RunContext[Deps], x: int) -> int:
    """Shape: Tool(fn) wrapper."""
    return x * 2


def toolset_fn(q: str) -> str:
    """Shape: FunctionToolset([fn])."""
    return q


def toolset_method_fn(q: str) -> str:
    """Shape: toolset.tool decorator-less add_function."""
    return q


toolset = FunctionToolset(tools=[toolset_fn])
toolset.add_function(toolset_method_fn)

agent = Agent(
    "openai:gpt-4o",
    deps_type=Deps,
    tools=[lookup_weather, Tool(tool_via_wrapper, takes_ctx=True)],
    toolsets=[toolset],
)


@agent.tool
async def get_user(ctx: RunContext[Deps], user_id: int) -> str:
    """Shape: @agent.tool decorator through a receiver."""
    return ctx.deps.db


@agent.tool_plain
def roll_dice() -> str:
    """Shape: @agent.tool_plain."""
    return "4"


@agent.tool(retries=2)
def with_args(ctx: RunContext[Deps]) -> str:
    """Shape: @agent.tool(...) with arguments."""
    return "x"


@toolset.tool
def decorated_toolset_tool(q: str) -> str:
    """Shape: @toolset.tool."""
    return q


@agent.system_prompt
def add_user_name(ctx: RunContext[Deps]) -> str:
    """Shape: dynamic system prompt."""
    return "name"


@agent.instructions
def add_instructions() -> str:
    return "be nice"


@agent.output_validator
async def validate_output(ctx: RunContext[Deps], output: str) -> str:
    if not output:
        raise ModelRetry("empty")
    return output


def never_used() -> None:
    """Control: genuinely dead."""


def main() -> None:
    agent.run_sync("hi", deps=Deps(db="x"))


if __name__ == "__main__":
    main()
"###,
    )]);
    assert_entries(&err, "pydantic-ai", "tool", 8);
    assert_entries(&err, "pydantic-ai-hooks", "lifecycle", 3);
    let _ = g;
}

#[test]
fn openai_agents_tools_guardrails_and_hooks() {
    let (g, err) = run(&[(
        "main.py",
        r###"import asyncio
from agents import (Agent, Runner, function_tool, handoff, input_guardrail, output_guardrail,
                    GuardrailFunctionOutput, RunHooks, AgentHooks, RunContextWrapper, FunctionTool,
                    InputGuardrail)


@function_tool
def get_weather(city: str) -> str:
    """Shape: bare @function_tool."""
    return city


@function_tool(name_override="fetch")
async def fetch_data(url: str) -> str:
    """Shape: @function_tool(...) with args."""
    return url


def plain_tool(x: int) -> int:
    """Shape: function_tool(fn) called."""
    return x


wrapped = function_tool(plain_tool)


async def on_invoke(ctx: RunContextWrapper, args: str) -> str:
    """Shape: FunctionTool(on_invoke_tool=fn)."""
    return args


manual = FunctionTool(name="m", description="d", params_json_schema={}, on_invoke_tool=on_invoke)


@input_guardrail
async def no_homework(ctx, agent, inp) -> GuardrailFunctionOutput:
    """Shape: @input_guardrail."""
    return GuardrailFunctionOutput(output_info=None, tripwire_triggered=False)


@output_guardrail
async def no_pii(ctx, agent, out) -> GuardrailFunctionOutput:
    return GuardrailFunctionOutput(output_info=None, tripwire_triggered=False)


async def raw_guardrail(ctx, agent, inp):
    """Shape: InputGuardrail(guardrail_function=fn)."""
    return GuardrailFunctionOutput(output_info=None, tripwire_triggered=False)


def on_handoff(ctx: RunContextWrapper) -> None:
    """Shape: handoff(on_handoff=fn)."""


def dynamic_instructions(ctx, agent) -> str:
    """Shape: Agent(instructions=fn)."""
    return "x"


class MyRunHooks(RunHooks):
    async def on_agent_start(self, context, agent):
        pass

    async def on_tool_end(self, context, agent, tool, result):
        pass


class MyAgentHooks(AgentHooks):
    async def on_start(self, context, agent):
        pass

    async def on_handoff(self, context, agent, source):
        pass


billing = Agent(name="billing", instructions="bill")
triage = Agent(
    name="triage",
    instructions=dynamic_instructions,
    tools=[get_weather, fetch_data, wrapped, manual],
    handoffs=[billing, handoff(billing, on_handoff=on_handoff)],
    input_guardrails=[no_homework, InputGuardrail(guardrail_function=raw_guardrail)],
    output_guardrails=[no_pii],
    hooks=MyAgentHooks(),
)


def never_used():
    """Control."""


async def main():
    await Runner.run(triage, "hi", hooks=MyRunHooks())


if __name__ == "__main__":
    asyncio.run(main())
"###,
    )]);
    assert_entries(&err, "openai-agents-python", "tool", 4);
    assert_entries(&err, "openai-agents-python-hooks", "lifecycle", 9);
    assert!(g.contains("tool::openai-agents-python::fetch"), "{g}");
}

#[test]
fn claude_agent_sdk_tools_and_hooks() {
    let (g, err) = run(&[(
        "main.py",
        r###"import anyio
from claude_agent_sdk import (tool, create_sdk_mcp_server, ClaudeAgentOptions, ClaudeSDKClient,
                              HookMatcher, query)


@tool("greet", "Greet a user", {"name": str})
async def greet_user(args):
    """Shape: @tool(name, desc, schema)."""
    return {"content": [{"type": "text", "text": args["name"]}]}


@tool("add", "Add", {"a": float, "b": float})
async def add(args):
    return {"content": []}


server = create_sdk_mcp_server(name="my-tools", version="1.0.0", tools=[greet_user, add])


async def check_bash(input_data, tool_use_id, context):
    """Shape: hook in HookMatcher(hooks=[fn])."""
    return {}


async def can_use(tool_name, input_data, context):
    """Shape: can_use_tool=fn."""
    return {"behavior": "allow"}


options = ClaudeAgentOptions(
    mcp_servers={"tools": server},
    allowed_tools=["mcp__tools__greet"],
    hooks={"PreToolUse": [HookMatcher(matcher="Bash", hooks=[check_bash])]},
    can_use_tool=can_use,
)


def never_used():
    """Control."""


async def main():
    async with ClaudeSDKClient(options=options) as client:
        await client.query("hi")


anyio.run(main)
"###,
    )]);
    assert_entries(&err, "claude-agent-sdk-python", "tool", 2);
    assert_entries(&err, "claude-agent-sdk-python-hooks", "lifecycle", 2);
    assert!(g.contains("tool::claude-agent-sdk-python::greet"), "{g}");
}

#[test]
fn bedrock_agentcore_runtime_handlers() {
    let (g, err) = run(&[(
        "main.py",
        r###"from bedrock_agentcore.runtime import BedrockAgentCoreApp, PingStatus
from bedrock_agentcore.identity.auth import requires_access_token
from strands import Agent

app = BedrockAgentCoreApp()
agent = Agent()


def format_reply(r):
    return {"result": str(r)}


@app.entrypoint
def invoke(payload, context):
    return format_reply(agent(payload.get("prompt")))


@app.ping
def health():
    return PingStatus.HEALTHY


def print_auth_url(url):
    print(url)


@requires_access_token(provider_name="google", scopes=["x"], auth_flow="USER_FEDERATION",
                       on_auth_url=print_auth_url)
async def need_token(*, access_token: str):
    return access_token


@app.websocket
async def ws_handler(websocket, context):
    await websocket.accept()


if __name__ == "__main__":
    app.run()
"###,
    )]);
    assert_entries(&err, "bedrock-agentcore-python", "network", 3);
    let _ = g;
}

#[test]
fn a2a_agent_executor() {
    let (g, err) = run(&[(
        "server.py",
        r###"from a2a.server.agent_execution import AgentExecutor, RequestContext
from a2a.server.events import EventQueue
from a2a.server.request_handlers import DefaultRequestHandler
from a2a.server.tasks import InMemoryTaskStore
from a2a.server.apps import A2AStarletteApplication
from a2a.types import AgentCard
import uvicorn


class HelloAgent:
    async def invoke(self):
        return "hello"


class HelloExecutor(AgentExecutor):
    def __init__(self):
        self.agent = HelloAgent()

    async def execute(self, context: RequestContext, event_queue: EventQueue) -> None:
        result = await self.agent.invoke()
        await event_queue.enqueue_event(result)

    async def cancel(self, context: RequestContext, event_queue: EventQueue) -> None:
        raise Exception("cancel not supported")


def main():
    card = AgentCard(name="hello")
    handler = DefaultRequestHandler(agent_executor=HelloExecutor(), task_store=InMemoryTaskStore())
    server = A2AStarletteApplication(agent_card=card, http_handler=handler)
    uvicorn.run(server.build())


if __name__ == "__main__":
    main()
"###,
    )]);
    assert_entries(&err, "a2a-python", "network", 2);
    let _ = g;
}

import type { ProjectGraph } from "../types/graph";

export const SAMPLE_PROJECT: ProjectGraph = {
  rootName: "task_pipeline",
  rootPath: "sample://task_pipeline",
  groups: [
    {
      id: "dir:.",
      kind: "directory",
      name: "task_pipeline",
      qualifiedName: "task_pipeline",
      parentId: null,
      filePath: null,
      isPackage: true,
      docstring: null,
      lineCount: 0,
    },
    {
      id: "dir:core",
      kind: "directory",
      name: "core",
      qualifiedName: "core",
      parentId: "dir:.",
      filePath: "core",
      isPackage: true,
      docstring: null,
      lineCount: 0,
    },
    {
      id: "dir:services",
      kind: "directory",
      name: "services",
      qualifiedName: "services",
      parentId: "dir:.",
      filePath: "services",
      isPackage: true,
      docstring: null,
      lineCount: 0,
    },
    {
      id: "file:main.py",
      kind: "module",
      name: "main.py",
      qualifiedName: "main",
      parentId: "dir:.",
      filePath: "main.py",
      isPackage: false,
      docstring: "Entrypoint for running the sample task pipeline.",
      lineCount: 28,
    },
    {
      id: "file:core/models.py",
      kind: "module",
      name: "models.py",
      qualifiedName: "core.models",
      parentId: "dir:core",
      filePath: "core/models.py",
      isPackage: false,
      docstring: "Domain models and validation helpers.",
      lineCount: 34,
    },
    {
      id: "class:core/models.py::TaskItem",
      kind: "class",
      name: "TaskItem",
      qualifiedName: "core.models.TaskItem",
      parentId: "file:core/models.py",
      filePath: "core/models.py",
      isPackage: false,
      docstring: "Represents a unit of work in the pipeline.",
      lineCount: 18,
    },
    {
      id: "file:services/runner.py",
      kind: "module",
      name: "runner.py",
      qualifiedName: "services.runner",
      parentId: "dir:services",
      filePath: "services/runner.py",
      isPackage: false,
      docstring: "Async task execution and retry orchestration.",
      lineCount: 40,
    },
    {
      id: "class:services/runner.py::PipelineRunner",
      kind: "class",
      name: "PipelineRunner",
      qualifiedName: "services.runner.PipelineRunner",
      parentId: "file:services/runner.py",
      filePath: "services/runner.py",
      isPackage: false,
      docstring: "Executes validated tasks with concurrency control.",
      lineCount: 26,
    },
  ],
  functions: [
    {
      id: "fn:main.py::bootstrap",
      name: "bootstrap",
      qualifiedName: "main.bootstrap",
      parentId: "file:main.py",
      filePath: "main.py",
      isAsync: true,
      isMethod: false,
      decorators: [],
      signature: "(raw_items: list[dict]) -> list[str]",
      docstring: "Parse tasks and run them through PipelineRunner.",
      startLine: 5,
      endLine: 16,
      code: `async def bootstrap(raw_items: list[dict]) -> list[str]:
    """Parse tasks and run them through PipelineRunner."""
    tasks = [build_task(item) for item in raw_items]
    runner = PipelineRunner(max_retries=2)
    return await runner.execute_batch(tasks)`,
    },
    {
      id: "fn:core/models.py::build_task",
      name: "build_task",
      qualifiedName: "core.models.build_task",
      parentId: "file:core/models.py",
      filePath: "core/models.py",
      isAsync: false,
      isMethod: false,
      decorators: [],
      signature: "(payload: dict) -> TaskItem",
      docstring: "Construct and validate a TaskItem from a dict.",
      startLine: 4,
      endLine: 12,
      code: `def build_task(payload: dict) -> TaskItem:
    """Construct and validate a TaskItem from a dict."""
    item = TaskItem(
        task_id=str(payload["id"]),
        priority=int(payload.get("priority", 1)),
    )
    item.validate()
    return item`,
    },
    {
      id: "fn:core/models.py::TaskItem.validate",
      name: "validate",
      qualifiedName: "core.models.TaskItem.validate",
      parentId: "class:core/models.py::TaskItem",
      filePath: "core/models.py",
      isAsync: false,
      isMethod: true,
      decorators: [],
      signature: "(self) -> None",
      docstring: "Ensure task_id is non-empty and priority is positive.",
      startLine: 18,
      endLine: 25,
      code: `def validate(self) -> None:
    """Ensure task_id is non-empty and priority is positive."""
    if not self.task_id.strip():
        raise ValueError("task_id must not be empty")
    if self.priority < 1:
        raise ValueError("priority must be >= 1")`,
    },
    {
      id: "fn:services/runner.py::PipelineRunner.execute_batch",
      name: "execute_batch",
      qualifiedName: "services.runner.PipelineRunner.execute_batch",
      parentId: "class:services/runner.py::PipelineRunner",
      filePath: "services/runner.py",
      isAsync: true,
      isMethod: true,
      decorators: [],
      signature: "(self, tasks: list[TaskItem]) -> list[str]",
      docstring: "Run all tasks in priority order.",
      startLine: 10,
      endLine: 21,
      code: `async def execute_batch(self, tasks: list[TaskItem]) -> list[str]:
    """Run all tasks in priority order."""
    ordered = sorted(tasks, key=lambda t: t.priority, reverse=True)
    results = []
    for task in ordered:
        results.append(await self._run_single(task))
    return results`,
    },
    {
      id: "fn:services/runner.py::PipelineRunner._run_single",
      name: "_run_single",
      qualifiedName: "services.runner.PipelineRunner._run_single",
      parentId: "class:services/runner.py::PipelineRunner",
      filePath: "services/runner.py",
      isAsync: true,
      isMethod: true,
      decorators: [],
      signature: "(self, task: TaskItem) -> str",
      docstring: "Execute a single task with retry handling.",
      startLine: 23,
      endLine: 34,
      code: `async def _run_single(self, task: TaskItem) -> str:
    """Execute a single task with retry handling."""
    for attempt in range(1, self.max_retries + 1):
        try:
            return format_result(task.task_id, attempt)
        except RuntimeError:
            if attempt == self.max_retries:
                raise
    return "unreachable"`,
    },
    {
      id: "fn:services/runner.py::format_result",
      name: "format_result",
      qualifiedName: "services.runner.format_result",
      parentId: "file:services/runner.py",
      filePath: "services/runner.py",
      isAsync: false,
      isMethod: false,
      decorators: [],
      signature: "(task_id: str, attempt: int) -> str",
      docstring: "Format completion status string.",
      startLine: 36,
      endLine: 40,
      code: `def format_result(task_id: str, attempt: int) -> str:
    """Format completion status string."""
    return f"completed:{task_id}:attempt={attempt}"`,
    },
  ],
  edges: [
    {
      id: "imp:main->models",
      kind: "import",
      sourceId: "file:main.py",
      targetId: "file:core/models.py",
      label: "imports",
    },
    {
      id: "imp:main->runner",
      kind: "import",
      sourceId: "file:main.py",
      targetId: "file:services/runner.py",
      label: "imports",
    },
    {
      id: "imp:runner->models",
      kind: "import",
      sourceId: "file:services/runner.py",
      targetId: "file:core/models.py",
      label: "imports",
    },
    {
      id: "call:bootstrap->build_task",
      kind: "call",
      sourceId: "fn:main.py::bootstrap",
      targetId: "fn:core/models.py::build_task",
      label: "build_task",
    },
    {
      id: "call:execute_batch->_run_single",
      kind: "call",
      sourceId: "fn:services/runner.py::PipelineRunner.execute_batch",
      targetId: "fn:services/runner.py::PipelineRunner._run_single",
      label: "_run_single",
    },
    {
      id: "call:_run_single->format_result",
      kind: "call",
      sourceId: "fn:services/runner.py::PipelineRunner._run_single",
      targetId: "fn:services/runner.py::format_result",
      label: "format_result",
    },
  ],
  parseErrors: [],
  stats: {
    totalFiles: 3,
    totalClasses: 2,
    totalFunctions: 6,
    totalImportEdges: 3,
    totalCallEdges: 3,
  },
};

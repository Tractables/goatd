from collections.abc import Sequence

__version__: str

class Error(Exception):
    """An invalid input, malformed PACE text, invalid decomposition, oversized
    problem, or failed FlowCutter construction."""

class Graph:
    """An undirected graph over the vertices `0..num_vertices`, as an edge list."""

    def __init__(
        self, num_vertices: int, edges: Sequence[tuple[int, int]]
    ) -> None: ...
    @staticmethod
    def from_gr(text: str) -> Graph: ...
    def to_gr(self) -> str: ...
    @property
    def num_vertices(self) -> int: ...
    @property
    def edges(self) -> list[tuple[int, int]]: ...

class TreeDecomposition:
    """Bags of graph vertices, and acyclic edges between the bags."""

    def __init__(
        self,
        graph: Graph,
        bags: Sequence[Sequence[int]],
        edges: Sequence[tuple[int, int]],
    ) -> None: ...
    @staticmethod
    def from_td(text: str) -> TreeDecomposition: ...
    def to_td(self) -> str: ...
    @property
    def num_vertices(self) -> int: ...
    @property
    def bags(self) -> list[list[int]]: ...
    @property
    def edges(self) -> list[tuple[int, int]]: ...
    @property
    def treewidth(self) -> int: ...
    @property
    def total_bag_size(self) -> int: ...
    def validate(self, graph: Graph) -> None: ...

def decompose(
    graph: Graph,
    *,
    order: str = "minfill",
    seed: int | None = None,
    ties: str | None = None,
    weights: Sequence[int] | None = None,
    budget_ms: int | None = None,
    steps: int | None = None,
    refine: bool = False,
) -> TreeDecomposition: ...
def refine_with_flowcutter(
    td: TreeDecomposition, graph: Graph, *, budget_ms: int | None = None
) -> TreeDecomposition: ...

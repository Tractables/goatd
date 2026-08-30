"""Tests for the goatd extension module, run against an installed wheel."""

from concurrent.futures import ThreadPoolExecutor

import pytest

import goatd

ORDERS = ["minfill", "mindegree", "nested-dissection", "flowcutter", "portfolio"]

# The 4-cycle with one chord: treewidth 2.
CHORDED_CYCLE = (4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)])


def grid(side):
    """The side x side grid graph, whose treewidth is `side`."""

    def vertex(row, column):
        return row * side + column

    edges = []
    for row in range(side):
        for column in range(side):
            if column + 1 < side:
                edges.append((vertex(row, column), vertex(row, column + 1)))
            if row + 1 < side:
                edges.append((vertex(row, column), vertex(row + 1, column)))
    return side * side, edges


def check(td, graph):
    """Check the tree-decomposition contract without asking goatd about it."""
    bags = td.bags
    tree_edges = td.edges
    num_vertices = graph.num_vertices
    assert td.num_vertices == num_vertices

    neighbours = {bag: set() for bag in range(len(bags))}
    for left, right in tree_edges:
        assert left < right < len(bags)
        neighbours[left].add(right)
        neighbours[right].add(left)

    # The bag graph is a forest: every component spans one more bag than it
    # has edges.
    components = 0
    seen = set()
    for start in range(len(bags)):
        if start in seen:
            continue
        components += 1
        stack = [start]
        seen.add(start)
        while stack:
            bag = stack.pop()
            for neighbour in neighbours[bag] - seen:
                seen.add(neighbour)
                stack.append(neighbour)
    assert len(tree_edges) == len(bags) - components

    # Every vertex is in a bag, and the bags holding it are connected.
    holders = {vertex: set() for vertex in range(num_vertices)}
    for position, bag in enumerate(bags):
        assert len(set(bag)) == len(bag)
        for vertex in bag:
            assert 0 <= vertex < num_vertices
            holders[vertex].add(position)
    for vertex, held in holders.items():
        assert held, f"vertex {vertex} is in no bag"
        stack = [next(iter(held))]
        reached = set(stack)
        while stack:
            bag = stack.pop()
            for neighbour in (neighbours[bag] & held) - reached:
                reached.add(neighbour)
                stack.append(neighbour)
        assert reached == held, f"the bags holding vertex {vertex} are not connected"

    # Every edge is inside a bag.
    for left, right in graph.edges:
        assert holders[left] & holders[right], f"edge ({left}, {right}) is in no bag"

    largest = max((len(bag) for bag in bags), default=0)
    assert td.treewidth == max(largest - 1, 0)
    assert td.total_bag_size == sum(len(bag) for bag in bags)


def test_module_surface():
    assert isinstance(goatd.__version__, str)
    assert issubclass(goatd.Error, Exception)
    assert set(goatd.__all__) == {
        "Error",
        "Graph",
        "TreeDecomposition",
        "__version__",
        "decompose",
        "refine_with_flowcutter",
    }


@pytest.mark.parametrize("order", ORDERS)
def test_chorded_cycle_has_width_two(order):
    graph = goatd.Graph(*CHORDED_CYCLE)
    td = goatd.decompose(graph, order=order, budget_ms=200)
    check(td, graph)
    td.validate(graph)
    assert td.treewidth == 2


@pytest.mark.parametrize("order", ORDERS)
def test_grid(order):
    graph = goatd.Graph(*grid(6))
    td = goatd.decompose(graph, order=order, budget_ms=2000)
    check(td, graph)
    # The 6x6 grid has treewidth 6, so no decomposition of it is narrower.
    assert 6 <= td.treewidth <= 12


def test_empty_graph():
    graph = goatd.Graph(0, [])
    td = goatd.decompose(graph)
    check(td, graph)
    assert td.treewidth == 0


def test_refinement():
    graph = goatd.Graph(*grid(6))
    td = goatd.decompose(graph, order="minfill", budget_ms=2000, refine=True)
    check(td, graph)
    again = goatd.refine_with_flowcutter(td, graph, budget_ms=500)
    check(again, graph)


def test_weighted_ties():
    num_vertices, edges = grid(5)
    graph = goatd.Graph(num_vertices, edges)
    td = goatd.decompose(
        graph,
        order="minfill",
        ties="sample",
        weights=list(range(1, num_vertices + 1)),
        seed=7,
    )
    check(td, graph)


def test_step_budget_repeats():
    graph = goatd.Graph(*grid(6))
    first = goatd.decompose(graph, order="flowcutter", steps=20_000)
    second = goatd.decompose(graph, order="flowcutter", steps=20_000)
    check(first, graph)
    assert first.to_td() == second.to_td()


def test_pace_round_trip():
    graph = goatd.Graph(*grid(4))
    reread = goatd.Graph.from_gr(graph.to_gr())
    assert reread.num_vertices == graph.num_vertices
    assert reread.edges == graph.edges

    td = goatd.decompose(graph, order="portfolio", budget_ms=500)
    assert td.to_td().startswith("s td ")
    parsed = goatd.TreeDecomposition.from_td(td.to_td())
    parsed.validate(graph)
    assert parsed.treewidth == td.treewidth


def test_decomposition_from_parts():
    graph = goatd.Graph(*CHORDED_CYCLE)
    td = goatd.TreeDecomposition(graph, [[0, 1, 2], [0, 2, 3]], [(0, 1)])
    check(td, graph)
    assert td.treewidth == 2
    with pytest.raises(goatd.Error):
        goatd.TreeDecomposition(graph, [[0, 1, 2], [0, 2]], [(0, 1)])


def test_library_errors_raise_goatd_error():
    with pytest.raises(goatd.Error):
        goatd.Graph(2, [(0, 5)])
    with pytest.raises(goatd.Error):
        goatd.Graph.from_gr("p tw 2 1\n1 9\n")
    with pytest.raises(goatd.Error):
        goatd.TreeDecomposition.from_td("not a decomposition")


@pytest.mark.parametrize(
    ("arguments", "expected"),
    [
        ({"order": "flowcutter", "seed": 1}, ("seed", "flowcutter")),
        ({"order": "portfolio", "ties": "sample"}, ("ties", "portfolio")),
        ({"order": "minfill", "steps": 10}, ("steps", "minfill")),
        ({"order": "minfill", "weights": [1, 1, 1, 1]}, ("weights", "sample")),
        ({"order": "flowcutter", "steps": 10, "budget_ms": 10}, ("steps", "budget_ms")),
        ({"order": "minfill", "ties": "salt"}, ("ties", "sample")),
        ({"order": "chordal"}, ("order", "minfill")),
        ({"budget_ms": 0}, ("budget_ms",)),
    ],
)
def test_arguments_the_order_cannot_act_on(arguments, expected):
    graph = goatd.Graph(*CHORDED_CYCLE)
    with pytest.raises(ValueError) as raised:
        goatd.decompose(graph, **arguments)
    message = str(raised.value)
    for word in expected:
        assert word in message


def test_threads_decompose_concurrently():
    graph = goatd.Graph(*grid(8))
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = [
            pool.submit(goatd.decompose, graph, order="minfill", seed=seed)
            for seed in range(8)
        ]
        decompositions = [result.result() for result in results]
    for td in decompositions:
        check(td, graph)
        assert td.treewidth >= 8

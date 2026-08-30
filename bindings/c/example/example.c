/* Decomposes a graph whose treewidth is known and checks the result.
 *
 * The graph is the complete graph on vertices 0..5 with a path 5-6-...-11
 * hanging off it. A clique has to sit inside one bag of any tree
 * decomposition, so every valid answer here has width at least 5, and the
 * graph is chordal, so an elimination order reaches exactly 5.
 *
 * Building and linking this program is covered in bindings/c/README.md. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "goatd.h"

#define REQUIRE(cond, ...)                                                     \
  do {                                                                         \
    if (!(cond)) {                                                             \
      fprintf(stderr, "example: ");                                            \
      fprintf(stderr, __VA_ARGS__);                                            \
      fprintf(stderr, "\n");                                                   \
      exit(1);                                                                 \
    }                                                                          \
  } while (0)

#define CLIQUE 6
#define NUM_VERTICES 12

/* Undirected edges as endpoint pairs: the clique first, then the path. */
static const uint32_t EDGES[] = {
    0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 1, 2, 1, 3, 1, 4,
    1, 5, 2, 3, 2, 4, 2, 5, 3, 4, 3, 5, 4, 5,
    5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11,
};
static const size_t NUM_EDGES = sizeof(EDGES) / (2 * sizeof(EDGES[0]));

/* Whether some bag holds both endpoints of an edge. */
static int edge_is_covered(const GoatdDecomposition *td, uint32_t u, uint32_t v) {
  for (size_t bag = 0; bag < td->num_bags; bag++) {
    int has_u = 0, has_v = 0;
    for (size_t i = td->bag_offsets[bag]; i < td->bag_offsets[bag + 1]; i++) {
      if (td->bag_vertices[i] == u) has_u = 1;
      if (td->bag_vertices[i] == v) has_v = 1;
    }
    if (has_u && has_v) return 1;
  }
  return 0;
}

/* The properties that make the arrays a tree decomposition of this graph,
 * checked here rather than taken on trust from the status code. */
static void check(const GoatdDecomposition *td, const char *what) {
  int seen[NUM_VERTICES];
  size_t widest = 0;

  REQUIRE(td->num_vertices == NUM_VERTICES, "%s: %u vertices, expected %d",
          what, td->num_vertices, NUM_VERTICES);
  REQUIRE(td->num_bags > 0, "%s: no bags", what);
  REQUIRE(td->bag_offsets[0] == 0, "%s: bag_offsets does not start at 0", what);

  memset(seen, 0, sizeof seen);
  for (size_t bag = 0; bag < td->num_bags; bag++) {
    size_t begin = td->bag_offsets[bag], end = td->bag_offsets[bag + 1];
    REQUIRE(begin <= end, "%s: bag %zu has a negative size", what, bag);
    if (end - begin > widest) widest = end - begin;
    for (size_t i = begin; i < end; i++) {
      uint32_t vertex = td->bag_vertices[i];
      REQUIRE(vertex < NUM_VERTICES, "%s: bag %zu holds vertex %u", what, bag,
              vertex);
      seen[vertex] = 1;
    }
  }
  for (uint32_t vertex = 0; vertex < NUM_VERTICES; vertex++)
    REQUIRE(seen[vertex], "%s: vertex %u is in no bag", what, vertex);

  REQUIRE(td->treewidth + 1 == widest, "%s: width %u beside a bag of %zu", what,
          td->treewidth, widest);

  for (size_t edge = 0; edge < NUM_EDGES; edge++) {
    uint32_t u = EDGES[2 * edge], v = EDGES[2 * edge + 1];
    REQUIRE(edge_is_covered(td, u, v), "%s: edge %u-%u is in no bag", what, u, v);
  }

  /* The graph is connected, so its bags have to form one tree. */
  REQUIRE(td->num_tree_edges == td->num_bags - 1,
          "%s: %zu bags joined by %zu edges", what, td->num_bags,
          td->num_tree_edges);
  for (size_t i = 0; i < 2 * td->num_tree_edges; i++)
    REQUIRE(td->tree_edges[i] < td->num_bags, "%s: tree edge names bag %zu",
            what, td->tree_edges[i]);

  /* goatd's own validator, over the same arrays. */
  REQUIRE(goatd_validate(NUM_VERTICES, EDGES, NUM_EDGES, td) == GOATD_OK,
          "%s: goatd_validate said %s", what, goatd_last_error_message());
}

/* Every vertex of the clique in one bag, which any valid answer must have. */
static int has_clique_bag(const GoatdDecomposition *td) {
  for (size_t bag = 0; bag < td->num_bags; bag++) {
    size_t begin = td->bag_offsets[bag], end = td->bag_offsets[bag + 1];
    int found = 0;
    for (size_t i = begin; i < end; i++)
      if (td->bag_vertices[i] < CLIQUE) found++;
    if (found == CLIQUE) return 1;
  }
  return 0;
}

static GoatdDecomposition run(GoatdOptions options, const char *what) {
  GoatdDecomposition td;
  GoatdStatus status =
      goatd_decompose(NUM_VERTICES, EDGES, NUM_EDGES, &options, &td);
  REQUIRE(status == GOATD_OK, "%s: status %d, %s", what, status,
          goatd_last_error_message());
  check(&td, what);
  REQUIRE(has_clique_bag(&td), "%s: no bag holds the whole clique", what);
  printf("%s: width %u, %zu bags\n", what, td.treewidth, td.num_bags);
  return td;
}

/* Options the chosen order cannot act on are rejected, not ignored. */
static void check_errors(void) {
  GoatdDecomposition td;
  GoatdOptions options = goatd_options_default();
  static const uint32_t bad_edges[] = {0, 1, 1, 99};
  GoatdStatus status;

  status = goatd_decompose(4, bad_edges, 2, &options, &td);
  REQUIRE(status == GOATD_ERROR_INVALID_INPUT, "an out-of-range endpoint gave %d",
          status);
  REQUIRE(strlen(goatd_last_error_message()) > 0,
          "an out-of-range endpoint gave no message");

  options = goatd_options_default();
  options.steps = 100;
  status = goatd_decompose(NUM_VERTICES, EDGES, NUM_EDGES, &options, &td);
  REQUIRE(status == GOATD_ERROR_INVALID_INPUT, "steps with min-fill gave %d",
          status);
  REQUIRE(strstr(goatd_last_error_message(), "steps") != NULL,
          "steps with min-fill gave: %s", goatd_last_error_message());
}

int main(void) {
  GoatdOptions options;
  GoatdDecomposition td;

  REQUIRE(strlen(goatd_version()) > 0, "no version string");
  printf("goatd %s\n", goatd_version());

  options = goatd_options_default();
  td = run(options, "min-fill");
  REQUIRE(td.treewidth == CLIQUE - 1, "min-fill: width %u on a chordal graph",
          td.treewidth);
  goatd_decomposition_free(&td);
  /* Freeing leaves the struct empty, so a second call is harmless. */
  goatd_decomposition_free(&td);

  options = goatd_options_default();
  options.order = GOATD_ORDER_FLOWCUTTER;
  options.steps = 1000;
  td = run(options, "flowcutter");
  REQUIRE(td.treewidth >= CLIQUE - 1, "flowcutter: width %u below the clique",
          td.treewidth);
  goatd_decomposition_free(&td);

  options = goatd_options_default();
  options.order = GOATD_ORDER_PORTFOLIO;
  options.budget_ms = 200;
  options.refine = true;
  td = run(options, "portfolio, refined");
  REQUIRE(td.treewidth >= CLIQUE - 1, "portfolio: width %u below the clique",
          td.treewidth);
  goatd_decomposition_free(&td);

  check_errors();

  printf("all checks passed\n");
  return 0;
}

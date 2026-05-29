# Regression fixture for union-typed values.schema.json fields.
#
# The chart's values.schema.json declares `port` as
# `["string","integer","null"]` with an integer default (8080). Before
# the union-codegen fix, the generated `schema Values` collapsed the
# union to its first member (`str`) while keeping the int default,
# emitting `port: str = 8080` — a type mismatch that aborted the
# wasm evaluator at schema instantiation even with `values = {}`.
#
# Rendering with an empty `values` exercises the schema's own default,
# which is exactly the path that used to abort.

import charts.unionchart as c

resources = c.template(c.TemplateOpts {
    values = {}
    release = "union"
})

# Regression fixture for values.schema.json defaults that contradict
# their emitted KCL field types.
#
# The chart schema below includes three unsafe defaults:
# - null default for a non-null KCL string type
# - numeric default for a string field
# - array default whose item does not match the item type
#
# Rendering with empty values exercises generated schema defaults. The
# unsafe defaults must be omitted before KCL instantiates the Values
# schema, while the matching `safe` default should still flow through.

import charts.defaults as c

resources = c.template(c.TemplateOpts {
    values = {}
    release = "defaults"
})

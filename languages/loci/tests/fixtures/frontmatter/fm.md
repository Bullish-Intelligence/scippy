---
type: note
single: "[[a]]"
list:
  - "[[b]]"
  - plain
  - "text [[c]] more"
nested:
  key: "[[a#A]]"
  deep: ["[[b]]"]
embed: "![[pic.png]]"
missing: "[[zzz]]"
flow: [[c]]
aliases:
  - Alt Name
  - 'Other'
tags: [alpha, 'beta/gamma']
number: 5
---
# Body
Body link [[a]] after
[//]: #   ^^^^^ reference scippy loci frontmatter . `a.md`/

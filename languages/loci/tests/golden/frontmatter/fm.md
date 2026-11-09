     ---
[//]:^ definition scippy loci frontmatter . `fm.md`/
[//]:  kind File
[//]:  display_name fm.md
     type: note
     single: "[[a]]"
[//]:         ^^^^^ reference scippy loci frontmatter . `a.md`/
     list:
       - "[[b]]"
[//]:     ^^^^^ reference scippy loci frontmatter . `b.md`/
       - plain
       - "text [[c]] more"
[//]:          ^^^^^ reference scippy loci frontmatter . `c.md`/
     nested:
       key: "[[a#A]]"
[//]:        ^^^^^^^ reference scippy loci frontmatter . `a.md`/A.
       deep: ["[[b]]"]
[//]:          ^^^^^ reference scippy loci frontmatter . `b.md`/
     embed: "![[pic.png]]"
[//]:        ^^^^^^^^^^^^ reference scippy loci frontmatter . `pic.png`/
     missing: "[[zzz]]"
     flow: [[c]]
[//]:      ^^^^^ reference scippy loci frontmatter . `c.md`/
     aliases:
       - Alt Name
[//]:    ^^^^^^^^ definition scippy loci frontmatter . `fm.md`/`alias:Alt Name`.
[//]:             kind Key
[//]:             display_name alias:Alt Name
[//]:             relationship scippy loci frontmatter . `fm.md`/ reference
       - 'Other'
[//]:    ^^^^^^^ definition scippy loci frontmatter . `fm.md`/`alias:Other`.
[//]:            kind Key
[//]:            display_name alias:Other
[//]:            relationship scippy loci frontmatter . `fm.md`/ reference
     tags: [alpha, 'beta/gamma']
[//]:       ^^^^^ reference scippy loci tags . alpha/
[//]:              ^^^^^^^^^^^^ reference scippy loci tags . beta/gamma/
     number: 5
     ---
     # Body
[//]:^^^^^^ definition scippy loci frontmatter . `fm.md`/Body.
[//]:       kind Namespace
[//]:       display_name Body
     Body link [[a]] after
[//]:          ^^^^^ reference scippy loci frontmatter . `a.md`/
     [//]: #   ^^^^^ reference scippy loci frontmatter . `a.md`/
     

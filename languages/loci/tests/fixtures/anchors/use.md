  Chain [[doc#Top#Child A]] resolves
[//]: # ^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`Child A`.
Deep chain [[doc#Top#Grandchild]] first descendant
[//]: #    ^^^^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/Grandchild.
Second chain [[doc#Other#Child A]] other branch
[//]: #      ^^^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`Child A~2`.
Broken chain [[doc#Other#Grandchild]] is missing
  Loose [[doc#Heading with punct]] ignores marks
[//]: # ^^^^^^^^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`Heading: with | punct`.
Wikilinks use no slug [[doc#my-heading]] so missing
A md slug [x](doc.md#my-heading) resolves
[//]: #   ^^^^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`My Heading!`.
Slug of a repeat [x](doc.md#repeat-1) is the second
[//]: #          ^^^^^^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`Repeat~2`.
A block [[doc#^b1]] resolves
[//]: # ^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`^b1`.
A dash block [[doc#^b-2]] resolves
[//]: #      ^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/`^b-2`.
Missing heading [[doc#Nope]] names the note
[//]: #         ^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/
Missing block [[doc#^nope]] names the note
[//]: #       ^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/
Trimmed [[ doc # Top ]] resolves
[//]: # ^^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/Top.
Repeated heading [[doc#Repeat]] is the first
[//]: #          ^^^^^^^^^^^^^^ reference scippy loci anchors . `doc.md`/Repeat.

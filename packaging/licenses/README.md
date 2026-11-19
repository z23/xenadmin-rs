# Standard licence texts

`scripts/collect-notices.py` puts these beside the dependency notices of a package for the
packages that declare a licence and ship no licence file of their own. The file name is the
SPDX identifier the package declares.

They are the standard texts, with the copyright line left as the licence's own placeholder:
what is known of the holders of such a package is the `authors` and `repository` in its
`package.json`. A package without a notice that declares an identifier with no text here
stops the collector; adding a text is a decision about distributing under that licence, so it
is the project owner's.

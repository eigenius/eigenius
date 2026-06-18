# ED Fig 5 IF lsmeans (D56 wrapped-R, emmeans over a D53 file-backed SampleSet):
# WRN loss activates the p53 DDR -- raised phospho-p53(S15) and p21 staining --
# selectively in MSI + TP53-proficient cells. p21 is a p53 transcriptional
# target, so it is recomputed over the MSI + p53-proficient lines; the p53-null
# MSI line (KM12) fails to induce it (the p53-independence control, ED Fig 5d).
# Inputs (D53 multi-file): [1] tidy per-cell IF intensities (175,974 cells),
# [2] Supp Table 1 (CCLE_MSI + TP53_status genotype).
ifp <- .Call("r_eigon_materialized_path", eigenius_inputs[[1]])
s1p <- .Call("r_eigon_materialized_path", eigenius_inputs[[2]])
suppressMessages({ library(emmeans) })
df <- read.csv(ifp, check.names = FALSE)
s1 <- read.csv(s1p, check.names = FALSE)
lk <- do.call(rbind, lapply(unique(df$cell_line), function(l) {
  r <- s1[grepl(paste0("^", l, "_"), s1$CCLE_ID), ][1, ]
  data.frame(cell_line = l, msi = r$CCLE_MSI, tp53 = r$TP53_status)
}))
df <- merge(df, lk, by = "cell_line"); df$lv <- log(df$value)
# Per-readout WRN_KO vs control least-squares-means contrast on log-intensity,
# adjusting for cell_line when the stratum has more than one line.
ct <- function(d) {
  d$condition <- factor(d$condition, levels = c("control", "WRN_KO"))
  m <- if (length(unique(d$cell_line)) > 1) {
    d$cell_line <- factor(d$cell_line); lm(lv ~ cell_line + condition, data = d)
  } else lm(lv ~ condition, data = d)
  summary(contrast(emmeans(m, ~condition), "revpairwise"))
}
pp  <- ct(df[df$readout == "p_p53" & df$msi == "MSI" & df$tp53 == "TP53_proficient", ])
p21 <- ct(df[df$readout == "p21"  & df$msi == "MSI" & df$tp53 == "TP53_proficient", ])
p21n <- ct(df[df$readout == "p21" & df$msi == "MSI" & df$tp53 == "TP53_null", ])
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:if_ed5:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:pp53_logfc", pp$estimate[1])
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:pp53_p_value", pp$p.value[1])
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:p21_logfc", p21$estimate[1])
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:p21_p_value", p21$p.value[1])
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:p21_null_logfc", p21n$estimate[1])
# Warrant: WRN loss activates the p53 response (BOTH p-p53 and p21 rise) in the
# MSI + p53-proficient stratum. p21 induction requires intact p53 -- the null
# stratum (p21_null_logfc <= 0) is the p53-independence control, not a failure.
if (pp$estimate[1] > 0 && pp$p.value[1] < 0.05 &&
    p21$estimate[1] > 0 && p21$p.value[1] < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:RaisesP53DamageMarkers", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

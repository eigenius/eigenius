# ED Fig 6c γH2AX intensity (D56 wrapped-R, emmeans over a D53 file-backed
# SampleSet): WRN loss raises nuclear γH2AX staining intensity MSI-selectively.
# γH2AX is the canonical DSB lesion marker (the paper names it before 53BP1), so
# this closes that leg of CausesDSBs alongside the 53BP1-foci warrant. ED 6c
# spans ES2 (MSS) + OVK18 (MSI); the MSI-vs-MSS difference in the WRN-KO-vs-
# control log10-intensity change is the paper's published statistic (contrast of
# least-squares means, P < 2e-16; mean log10 FC 0.147 OVK18 / 0.055 ES2).
# Inputs (D53 multi-file): [1] tidy per-cell γH2AX intensities (32,882 cells),
# [2] Supp Table 1 (CCLE_MSI genotype).
ifp <- .Call("r_eigon_materialized_path", eigenius_inputs[[1]])
s1p <- .Call("r_eigon_materialized_path", eigenius_inputs[[2]])
suppressMessages({ library(emmeans) })
df <- read.csv(ifp, check.names = FALSE)
s1 <- read.csv(s1p, check.names = FALSE)
lk <- do.call(rbind, lapply(unique(df$cell_line), function(l) {
  r <- s1[grepl(paste0("^", l, "_"), s1$CCLE_ID), ][1, ]
  data.frame(cell_line = l, msi = r$CCLE_MSI)
}))
df <- merge(df, lk, by = "cell_line")
df$lv <- log10(df$value)
df$condition <- factor(df$condition, levels = c("control", "WRN_KO"))
df$msi <- factor(df$msi, levels = c("MSS", "MSI"))
# WRN_KO vs control least-squares-means contrast per MSI stratum (log10), and the
# MSI-vs-MSS interaction (the published ED-6c contrast).
m <- lm(lv ~ msi * condition, data = df)
pl <- summary(contrast(emmeans(m, ~ condition | msi), "revpairwise"))
ic <- summary(contrast(emmeans(m, ~ condition * msi), interaction = "revpairwise"))
msi_fc <- pl$estimate[pl$msi == "MSI"][1]
mss_fc <- pl$estimate[pl$msi == "MSS"][1]
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:gh2ax:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_logfc_msi", msi_fc)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_logfc_mss", mss_fc)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_interaction_p", ic$p.value[1])
# Warrant: WRN loss raises γH2AX in MSI (msi_fc > 0) AND more than in MSS
# (positive, significant interaction) — MSI-selective DSB induction.
if (msi_fc > 0 && ic$estimate[1] > 0 && ic$p.value[1] < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:CausesDSBs", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

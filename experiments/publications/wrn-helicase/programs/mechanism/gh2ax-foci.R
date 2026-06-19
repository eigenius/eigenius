# DSB induction, γH2AX foci (ED Fig 6a/6d): WRN loss raises nuclear γH2AX foci
# per cell MSI-SELECTIVELY (D56 wrapped-R over a D53 file-backed SampleSet).
# 94,791 cells across colon SW620/KM12/SW48 + ovarian ES2/OVK18. γH2AX is the
# diffuse DSB marker; saturated (pan-nuclear) cells — the most-damaged, enriched
# in MSI WRN-KO — are counted at a saturation ceiling in the slice rather than
# dropped (see extract/gh2ax-ed6ad-extract.R), so the foci readout is unbiased.
# The discrete-foci view complements the ED-6c intensity warrant; together they
# give γH2AX two independent recomputed legs of CausesDSBs. The MSI-selectivity
# is an interaction (condition x MSI), so this is wrapped-R lm.
# Inputs: [1] tidy per-cell γH2AX foci, [2] Supp Table 1 (MSI).
fp  <- .Call("r_eigon_materialized_path", eigenius_inputs[[1]])
s1p <- .Call("r_eigon_materialized_path", eigenius_inputs[[2]])
df <- read.csv(fp, check.names = FALSE)
s1 <- read.csv(s1p, check.names = FALSE)
lk <- do.call(rbind, lapply(unique(df$cell_line), function(l) {
  r <- s1[grepl(paste0("^", l, "_"), s1$CCLE_ID), ][1, ]
  data.frame(cell_line = l, msi = r$CCLE_MSI)
}))
df <- merge(df, lk, by = "cell_line")
df$condition <- factor(df$condition, levels = c("control", "WRN_KO"))
df$msi <- factor(df$msi, levels = c("MSS", "MSI"))
df$cell_line <- factor(df$cell_line)
m <- lm(value ~ cell_line + condition * msi, data = df)
co <- summary(m)$coefficients
ix <- grep("conditionWRN_KO:msiMSI", rownames(co))
inter_est <- co[ix, 1]; inter_p <- co[ix, 4]
fc <- function(s) { d <- df[df$msi == s, ]; mean(d$value[d$condition == "WRN_KO"]) / mean(d$value[d$condition == "control"]) }
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:gh2ax_foci:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_foci_interaction", inter_est)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_foci_interaction_p", inter_p)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_foci_fc_msi", fc("MSI"))
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:gh2ax_foci_fc_mss", fc("MSS"))
# Warrant: WRN loss raises γH2AX foci MORE in MSI than MSS (positive, significant
# interaction) -- MSI-selective DSB induction (discrete-foci leg).
if (inter_est > 0 && inter_p < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:CausesDSBs", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

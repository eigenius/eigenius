# DSB induction (ED Fig 6f/6h): WRN loss raises Apple-53BP1-trunc DSB foci per
# cell, MSI-SELECTIVELY (D56 wrapped-R over a D53 file-backed SampleSet). 39,249
# cells across SW620/ES2 (MSS) + KM12/OVK18 (MSI). The MSI-selectivity is an
# interaction (condition x MSI), so this is wrapped-R lm, not the institution's
# additive ANOVA. Inputs: [1] tidy per-cell foci counts, [2] Supp Table 1 (MSI).
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
# foci ~ cell_line + condition*MSI: the interaction is the MSI-selective extra
# DSB induction on WRN loss (above the MSS baseline change).
m <- lm(value ~ cell_line + condition * msi, data = df)
co <- summary(m)$coefficients
ix <- grep("conditionWRN_KO:msiMSI", rownames(co))
inter_est <- co[ix, 1]; inter_t <- co[ix, 3]; inter_p <- co[ix, 4]
# Per-stratum WRN-KO vs control mean-fold-change (descriptive measurements).
fc <- function(s) { d <- df[df$msi == s, ]; mean(d$value[d$condition == "WRN_KO"]) / mean(d$value[d$condition == "control"]) }
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:foci_dsb:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:dsb_msi_interaction", inter_est)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:dsb_msi_interaction_p", inter_p)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:dsb_foci_fc_msi", fc("MSI"))
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:dsb_foci_fc_mss", fc("MSS"))
# Warrant: WRN loss induces DSB foci MORE in MSI than MSS (positive, significant
# interaction) -- MSI-selective DSB induction.
if (inter_est > 0 && inter_p < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:CausesDSBs", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

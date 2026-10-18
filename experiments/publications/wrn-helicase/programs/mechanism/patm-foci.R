# DDR signaling (ED Fig 7b/7d): WRN loss raises phospho-ATM(S1981) foci per cell,
# MSI-SELECTIVELY (D56 wrapped-R over a D53 file-backed SampleSet). 191,241 cells
# across SW620/ES2 (MSS) + KM12/SW48/OVK18 (MSI). pATM(S1981) autophosphorylation
# reports activation of the apical ATM DSB-response kinase — the signaling step
# the paper uses to bridge DSBs -> p53 / anti-proliferative signaling. The
# MSI-selectivity is an interaction (condition x MSI), so this is wrapped-R lm.
# Inputs: [1] tidy per-cell pATM foci counts, [2] Supp Table 1 (MSI).
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
# pATM activation on WRN loss (above the MSS baseline change).
m <- lm(value ~ cell_line + condition * msi, data = df)
co <- summary(m)$coefficients
ix <- grep("conditionWRN_KO:msiMSI", rownames(co))
inter_est <- co[ix, 1]; inter_p <- co[ix, 4]
fc <- function(s) { d <- df[df$msi == s, ]; mean(d$value[d$condition == "WRN_KO"]) / mean(d$value[d$condition == "control"]) }
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:patm:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:patm_msi_interaction", inter_est)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:patm_msi_interaction_p", inter_p)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:patm_foci_fc_msi", fc("MSI"))
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:patm_foci_fc_mss", fc("MSS"))
# Warrant: WRN loss activates the ATM DSB-response MORE in MSI than MSS (positive,
# significant interaction) -- MSI-selective DDR signaling, the bridge to p53.
if (inter_est > 0 && inter_p < 0.05) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:ActivatesDSBResponse", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

# Paralogue co-loss control (ED Fig 9a): WRN's MSI-selective dependence is
# INTRINSIC to MSI, not a confound of paralogue / DNA-helicase co-loss. D56
# wrapped-R over the 1.6 GB DepMap omics rds (D53 PinnedExternalFile, read via
# readRDS) -- the large multi-schema container path. Inputs: [1] DepMap rds,
# [2] Supp Table 1 (avg_WRN_dep + CCLE_MSI).
rp  <- .Call("r_eigon_materialized_path", eigenius_inputs[[1]])
s1p <- .Call("r_eigon_materialized_path", eigenius_inputs[[2]])
dat <- readRDS(rp)
s1  <- read.csv(s1p, check.names = FALSE)
cd  <- data.frame(CCLE_ID = s1$CCLE_ID, MSI = s1$CCLE_MSI == "MSI",
                  avg_WRN_dep = suppressWarnings(as.numeric(s1$avg_WRN_dep)))
CN_loss <- -1; GE_un <- 1   # authors' thresholds (methylation ignored: Inf)
cls <- cd$CCLE_ID[!is.na(cd$avg_WRN_dep) & cd$CCLE_ID %in% rownames(dat$GE) & cd$CCLE_ID %in% rownames(dat$CN)]
gene_loss <- function(g) {
  il <- setNames(rep(FALSE, length(cls)), cls)
  if (!is.null(dat$MUT_DAM) && g %in% colnames(dat$MUT_DAM)) { u <- intersect(cls, rownames(dat$MUT_DAM)); il[u[which(dat$MUT_DAM[u, g] == TRUE)]] <- TRUE }
  if (g %in% colnames(dat$CN)) { u <- intersect(cls, rownames(dat$CN)); v <- dat$CN[u, g]; il[u[which(v < CN_loss & !is.na(v))]] <- TRUE }
  if (g %in% colnames(dat$GE)) { u <- intersect(cls, rownames(dat$GE)); v <- dat$GE[u, g]; il[u[which(v < GE_un & !is.na(v))]] <- TRUE }
  il
}
base <- summary(lm(avg_WRN_dep ~ MSI, data = cd))$coefficients
base_b <- base["MSITRUE", 1]; base_p <- base["MSITRUE", 4]
# Control for each RECQ paralogue's loss; the MSI coefficient must survive.
paralogues <- c("RECQL", "BLM", "RECQL4", "RECQL5")
msi_bs <- c(); msi_ps <- c()
for (g in paralogues) {
  gl <- gene_loss(g)
  df <- merge(cd, data.frame(CCLE_ID = names(gl), gene_loss = gl), by = "CCLE_ID")
  co <- summary(lm(avg_WRN_dep ~ MSI + gene_loss, data = df))$coefficients
  msi_bs <- c(msi_bs, co["MSITRUE", 1]); msi_ps <- c(msi_ps, co["MSITRUE", 4])
}
worst_p <- max(msi_ps)            # weakest MSI coefficient across paralogue controls
same_sign <- all(sign(msi_bs) == sign(base_b))
b <- .Call("r_eigon_begin", "urn:eigenius:pub:wrn:paralog_ctrl:result")
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:paralog_baseline_msi_beta", base_b)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:paralog_baseline_msi_p", base_p)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:paralog_controlled_msi_p_max", worst_p)
.Call("r_eigon_set_f64", b, "urn:eigenius:pub:wrn:paralog_controlled_msi_beta_min", min(msi_bs))
# Warrant: the MSI coefficient stays significant and same-signed controlling for
# every paralogue's loss -- WRN dependence is not explained by paralogue co-loss.
if (base_p < 0.05 && worst_p < 0.05 && same_sign) {
  .Call("r_eigon_set_proposition", b,
        "urn:eigenius:benchmark:onco:NotExplainedByParalogLoss", c("WRN", "MSI"))
}
.Call("r_eigon_finish", b)

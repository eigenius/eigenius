suppressMessages({library(data.table); library(limma)})
dt  <- fread("achilles_18Q4_gene_effect.csv")
ids <- dt[[1]]; M <- as.matrix(dt[, -1]); rownames(M) <- ids
si  <- fread("achilles_18Q4_sample_info.csv")
s1  <- fread("wrn_supplementary_table_1.csv")
ccle <- si$CCLE_name[match(ids, si$DepMap_ID)]
msi  <- s1$CCLE_MSI[match(ccle, s1$CCLE_ID)]
keep <- !is.na(msi) & msi %in% c("MSI","MSS")
M <- M[keep,]; grp <- factor(msi[keep], levels=c("MSS","MSI"))
cat(sprintf("cell lines: %d (MSI %d / MSS %d), genes: %d\n", nrow(M), sum(grp=="MSI"), sum(grp=="MSS"), ncol(M)))
# limma: features (genes) x samples (cell lines)
expr <- t(M)
design <- model.matrix(~ grp)            # coef 2 = MSI vs MSS
fit <- eBayes(lmFit(expr, design))
tt  <- topTable(fit, coef=2, number=Inf, sort.by="none")
tt$gene <- rownames(tt)
# MSI-preferential = more essential in MSI = logFC < 0 (lower CERES); rank by P
neg <- tt[tt$logFC < 0, ]; neg <- neg[order(neg$P.Value), ]
cat("Top 5 MSI-preferential by limma moderated-t:\n")
print(head(neg[,c("gene","logFC","t","P.Value","adj.P.Val")],5))
w <- which(grepl("^WRN \\(", neg$gene))[1]
cat(sprintf("\nWRN rank among MSI-preferential: %d of %d\n", w, nrow(neg)))
cat(sprintf("WRN: logFC=%.3f  modt=%.2f  P=%.3g  adj.P(Q)=%.3g\n",
            neg$logFC[w], neg$t[w], neg$P.Value[w], neg$adj.P.Val[w]))

# Contributor licence agreement (CLA) proposal

> **DRAFT PROPOSAL. NOT IN FORCE.**
>
> - It is not legal advice and it has not been reviewed by a lawyer. It needs that review.
> - It also needs the approval of the project owner. Until it is approved it is not used.
> - Until it is approved, **no external contribution is accepted** (ADR-0019; decision D15 of the [Stage 9 record](../architecture/stage9-record.md)).
> - No external contribution has been accepted to date, and this draft does not regularise any.
> - It is not linked from the README as if it were in force, and no `CONTRIBUTING.md` refers to it.
> - Direction (b) confirmed by the owner on 2026-11-19; the text is still pending legal review and is not in force. See [docs/owner-decisions.md](../owner-decisions.md).

| Field | Value |
|---|---|
| Status | Draft proposal, not in force |
| Date | 2026-11-13 (drafting revision 2026-11-20) |
| Origin | [ADR-0019](../adr/0019-license-and-contributions.md), decision D15, direction (b): a broad non-exclusive licence with the right to relicense, without assignment of ownership |
| Pending | Review by a lawyer and approval by the owner |

Text in [square brackets] is an open point for the owner or the lawyer. It is not part of the agreement.

## 1. Definitions and scope

1. **"Project"** means Kerno.
2. **"Owner"** means [full legal name, type of person, tax ID and domicile, to be declared by the owner], and its successors and assigns.
3. **"You"** means the person who Submits a Contribution or, if that person acts for a legal entity that holds the rights in it, that entity.
4. **"Contribution"** means any work of authorship (code, documentation or other material, including a change to existing material) that You intentionally Submit to the Project for inclusion in it.
5. **"Submit"** means to send to the Owner or to the Project through any channel the Owner manages, such as a pull request, a patch, the issue tracker or email. Material that You mark clearly in writing as "Not a Contribution" is not Submitted.

This agreement covers every Contribution You Submit while it is in force.

## 2. Copyright licence

1. **No assignment.** You keep the ownership of the copyright in your Contribution. This agreement is a licence. It transfers no ownership.
2. **Grant.** You grant the Owner a **non-exclusive, worldwide, royalty-free, irrevocable** licence, for the full term of the rights in the Contribution including any extension, to reproduce, modify, adapt, translate, prepare derivative works of, distribute, publicly perform, publicly display, communicate and make available to the public the Contribution and those derivative works. The licence covers any medium or format now known or later devised, source and object form, and an unlimited number of copies and uses.
3. **Sublicensing and relicensing.** The Owner may **sublicense** these rights through multiple tiers, and may **license the Contribution under any terms it chooses**, including proprietary or commercial terms that differ from the Project's public licence.
4. **Transfer.** The Owner may transfer this licence, whole, to whoever succeeds it as holder of the Project.
5. **Public licence.** You also license the Contribution to everyone who receives the Project, under the Project's public licence in force when You Submit (today MIT OR Apache-2.0, as stated in its README and `Cargo.toml`). This agreement does not limit that public licence. It gives the Owner the additional rights in this section.
   [Owner decision: whether the Owner also commits to keep each Contribution, as accepted, available under that public licence. Without that commitment clause 3 lets the Owner publish future versions under other terms only.]
6. **Moral rights.** You keep the moral rights the law gives You. To the extent the law allows, You agree not to assert them in a way that prevents the exercise of the licences in this agreement. The Owner credits contributors according to the Project's practice, such as the commit history.
   [Lawyer: effect of this clause where moral rights cannot be waived.]
7. **No duty to use.** The Owner decides whether to include a Contribution in the Project and may remove it later.

## 3. Patent licence

1. You grant the Owner, and everyone who receives the Project from the Owner, a non-exclusive, worldwide, royalty-free, irrevocable (except as stated in clause 3.3) patent licence to make, have made, use, offer to sell, sell, import and otherwise transfer the Contribution.
2. The licence covers only the patent claims that You own or can license and that are necessarily infringed by your Contribution alone, or by its combination with the Project as it stood when You Submitted it.
3. If a person starts patent litigation against You or against anyone else, including a cross-claim or counterclaim, alleging that the Contribution or the Project that contains it infringes a patent, the patent licences that this agreement gives that person for that Contribution end on the date the litigation is filed.

[Scope of this section, pending review by the lawyer.]

## 4. Statements of the contributor

You state that:

a) You have the legal capacity to enter into this agreement and, if You act for a legal entity, the authority to bind it;

b) You have the right to grant the licences in sections 2 and 3. If an employer or another third party has rights in what You contribute, You have its written permission to make the Contribution under this agreement, or it has accepted this agreement itself;

c) the Contribution is your own original work. Any part that is not yours is identified clearly in the submission, with its origin, its licence and any restriction You know of. That part is not licensed by You under section 2: it comes under its own licence, which must be compatible with the Project's public licence;

d) You know of no claim, litigation or third-party right that affects the Contribution, except any You point out in the submission;

e) [Owner decision: a statement on material produced with generative tools, for example that You disclose it in the submission and that the terms of the tool do not restrict the licences in this agreement];

f) You will tell the Owner promptly if You learn that any of these statements is no longer accurate.

## 5. Rights you keep

You keep all rights in the Contribution that this agreement does not grant. You may use it, publish it or license it to third parties under any terms. Nothing in this agreement obliges You to license it exclusively to anyone. The only limit is that the non-exclusive licences in sections 2 and 3 stay in place.

## 6. No warranty and no support

Except for the statements in section 4, and to the extent the law allows, the Contribution is provided **as is**, without warranty of any kind, express or implied, including warranties of title, non-infringement, merchantability and fitness for a particular purpose. You have no duty to maintain or support the Contribution.

## 7. How agreement is given

1. You accept this agreement, in the version in force on the date You Submit, in one of these ways:

   a) a statement in the description of the pull request: `I accept the Kerno Contributor Licence Agreement, version <number>.`; or

   b) a `Signed-off-by: <name> <email>` line at the end of every commit of the Contribution. This way is valid only while the Project's `CONTRIBUTING.md` states that this line means acceptance of this agreement in the version in force.
   [Lawyer and owner: in common practice this line certifies the Developer Certificate of Origin, which grants none of the rights in section 2. Option (b) is the weaker proof of consent. Consider keeping (a) alone, or a CLA tool that records an explicit acceptance.]

2. Acceptance by either way has the same effect as a handwritten signature.
3. The Owner keeps a record of each acceptance: name, email or account, version accepted, date and a link to the submission. It uses that record only to administer this agreement.
   [Lawyer: personal data notice for this record.]

## 8. Versions and entry into force

1. This agreement enters into force when the Owner approves it in writing, with a version number and a date. It applies only to Contributions Submitted from that date.
2. Any later change to the text is a new version, with its own number and date.
3. Each Contribution is governed by the version in force on the date it was Submitted. A later version does not change the licences already granted.

## 9. General

1. This agreement is the whole agreement between You and the Owner about your Contributions.
2. If a clause is held invalid, the rest stays in force, and the clause is applied to the greatest extent the law allows.
3. This agreement creates no employment, partnership or agency relationship, and no payment is owed for a Contribution.
4. [If the agreement is published in more than one language: which version prevails.]

## 10. Governing law and jurisdiction

[Pending review by the lawyer. To decide: the governing law, the competent courts, and how the agreement operates for contributors who are consumers or who live in other countries.]

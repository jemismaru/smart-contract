use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::system_instruction;
use std::collections::HashMap;

declare_id!("YourProgramIdHere");

#[program]
pub mod nft_com_auction {
    use super::*;

    pub fn change_fee_recipient(
        ctx: Context<ChangeFeeRecipient>,
        new_fee_recipient: Pubkey
    ) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        auction_state.fee_recipient = new_fee_recipient;
        Ok(())
    }

    // Change the NFT contract address
    pub fn change_nft_contract(
        ctx: Context<ChangeNFTContract>,
        new_nft_contract: Pubkey
    ) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        auction_state.nft_contract = new_nft_contract;
        Ok(())
    }

    // Set buyer and seller fees
    pub fn set_fees(ctx: Context<SetFees>, buyer_fee: u64, seller_fee: u64) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        auction_state.buyer_fee = buyer_fee;
        auction_state.seller_fee = seller_fee;
        Ok(())
    }

    // Emergency pause auction
    pub fn emergency_pause_auction(
        ctx: Context<EmergencyPauseAuction>,
        listing_id: String,
        status: bool
    ) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        let auction = auction_state.auctions
            .get_mut(&listing_id)
            .ok_or(ErrorCode::InvalidListingId)?;
        auction.paused = status;
        Ok(())
    }

    // Initialize auction
    pub fn initialize_auction(
        ctx: Context<InitializeAuction>,
        listing_id: String,
        minimum: u64,
        end_time: i64,
        owner: Pubkey,
        bidder: Option<Pubkey>
    ) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;

        let bidder = bidder.unwrap_or(ctx.accounts.owner.key());

        if auction_state.auctions.contains_key(&listing_id) {
            return Err(ErrorCode::InvalidListingId.into());
        }
        require!(minimum > 0, ErrorCode::MinimumBidError);
        require!(end_time > Clock::get().unwrap().unix_timestamp, ErrorCode::EndTimeError);

        let auction = AuctionDetails {
            listing_id: listing_id.clone(),
            highest_bid: 0,
            highest_bidder: Pubkey::default(),
            minimum_bid: minimum,
            end_time,
            fees: 0,
            ended: false,
            paused: false,
            is_alien: false,
            total_amount: 0,
            owner,
            bidders: vec![],
        };

        auction_state.auctions.insert(listing_id.clone(), auction);
        auction_state.active_auctions.entry(owner).or_default().push(listing_id.clone());
        place_bid(ctx, listing_id, bidder)?;
        emit!(AuctionInitialized { listing_id, minimum, end_time });
        Ok(())
    }

    // Place a bid
    pub fn place_bid(ctx: Context<PlaceBid>, listing_id: String, bidder: Pubkey) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        let auction = auction_state.auctions
            .get_mut(&listing_id)
            .ok_or(ErrorCode::InvalidListingId)?;

        require!(bidder != auction.owner, ErrorCode::BidderIsOwner);
        require!(ctx.accounts.owner.key() != auction.owner, ErrorCode::BidderIsOwner);

        let fee = (ctx.accounts.bid_amount * auction_state.buyer_fee) / 1000;
        let bid_amount = ctx.accounts.bid_amount - fee;

        require!(!auction.ended, ErrorCode::AuctionEnded);
        require!(!auction.paused, ErrorCode::AuctionPaused);
        require!(Clock::get().unwrap().unix_timestamp <= auction.end_time, ErrorCode::AuctionEnded);

        // Check for sniping protection
        if
            Clock::get().unwrap().unix_timestamp >=
            auction.end_time - auction_state.sniping_time_window
        {
            auction.end_time += auction_state.time_extension;
        }

        auction.total_amount += bid_amount;

        // Update highest bid logic
        // (Similar to the original logic...)

        emit!(BidPlaced { listing_id, sender: bidder, value: bid_amount });
        Ok(())
    }

    pub fn withdraw(
        ctx: Context<Withdraw>,
        listing_id: String,
        to: Option<Pubkey>
    ) -> ProgramResult {
        let auction_state = &mut ctx.accounts.auction_state;
        let auction = auction_state.auctions
            .get_mut(&listing_id)
            .ok_or(ErrorCode::InvalidListingId)?;

        // Ensure auction is not an "alien" auction
        require!(!auction.is_alien, ErrorCode::AlienAuctionError);

        // Ensure the caller is not the highest bidder
        require!(
            ctx.accounts.bidder.key() != auction.highest_bidder,
            ErrorCode::HighestBidderCannotWithdraw
        );

        // Get the refund amount
        let refund_amount = auction.bidders
            .iter()
            .find(|b| b.key == ctx.accounts.bidder.key())
            .ok_or(ErrorCode::NoFundsToWithdraw)?.amount;

        // Ensure the refund amount is greater than 0
        require!(refund_amount > 0, ErrorCode::NoFundsToWithdraw);

        // Process refund (handle case for `to` address)
        let recipient = to.unwrap_or(ctx.accounts.bidder.key());

        **ctx.accounts.bidder.try_borrow_mut_lamports()? -= refund_amount;
        **ctx.accounts.to.try_borrow_mut_lamports()? += refund_amount;

        // Update the bidder's amount to 0 after withdrawal
        auction.bidders
            .iter_mut()
            .find(|b| b.key == ctx.accounts.bidder.key())
            .unwrap().amount = 0;

        Ok(())
    }

    pub fn get_user_bid(
        ctx: Context<GetUserBid>,
        listing_id: String,
        user: Pubkey
    ) -> Result<(Pubkey, u64, i64), ProgramError> {
        let auction_state = &ctx.accounts.auction_state;
        let auction = auction_state.auctions.get(&listing_id).ok_or(ErrorCode::InvalidListingId)?;

        if let Some(bid) = auction.bidders.iter().find(|b| b.key == user) {
            return Ok((user, bid.amount, bid.time));
        }

        Ok((Pubkey::default(), 0, 0))
    }

    pub fn get_all_bids_of_user(
        ctx: Context<GetAllBidsOfUser>,
        bidder: Pubkey
    ) -> Result<(Vec<String>, Vec<u64>, Vec<i64>), ProgramError> {
        let auction_state = &ctx.accounts.auction_state;
        let active_bids_for_user = auction_state.active_bids.get(&bidder).unwrap_or(&vec![]);

        let mut amounts = vec![];
        let mut times = vec![];

        for listing_id in active_bids_for_user.iter() {
            if let Some(auction) = auction_state.auctions.get(listing_id) {
                let bid = auction.bidders
                    .iter()
                    .find(|b| b.key == bidder)
                    .unwrap();
                amounts.push(bid.amount);
                times.push(bid.time);
            }
        }

        Ok((active_bids_for_user.clone(), amounts, times))
    }

    pub fn get_latest_bids(
        ctx: Context<GetLatestBids>,
        listing_id: String,
        n: u64
    ) -> Result<(Vec<Pubkey>, Vec<u64>, Vec<i64>), ProgramError> {
        let auction_state = &ctx.accounts.auction_state;
        let auction = auction_state.auctions.get(&listing_id).ok_or(ErrorCode::InvalidListingId)?;

        let length = auction.bidders.len();
        let n = if (n as usize) > length { length } else { n as usize };

        let mut latest_bidders = vec![];
        let mut latest_bid_amounts = vec![];
        let mut latest_bid_times = vec![];

        for i in 0..n {
            let bidder = &auction.bidders[length - 1 - i];
            latest_bidders.push(bidder.key);
            latest_bid_amounts.push(bidder.amount);
            latest_bid_times.push(bidder.time);
        }

        Ok((latest_bidders, latest_bid_amounts, latest_bid_times))
    }

    pub fn end_auction(ctx: Context<EndAuction>, listing_id: String, hook: Pubkey) -> Result<()> {
        let auction_state = &mut ctx.accounts.auction_state;
        let auction = auction_state.auctions
            .get_mut(&listing_id)
            .ok_or(ErrorCode::InvalidListingId)?;

        // Ensure auction has ended
        let clock = Clock::get().unwrap();
        require!(clock.unix_timestamp >= auction.end_time, ErrorCode::AuctionNotEnded);
        require!(!auction.ended, ErrorCode::AuctionAlreadyEnded);
        require!(auction.highest_bid > 0, ErrorCode::NothingToWithdraw);

        auction.ended = true;

        // Calculate fees and owner earnings
        let seller_fee = auction_state.seller_fee;
        let mut fee = (auction.highest_bid * seller_fee) / 1000;
        let mut owner_earnings = auction.highest_bid - fee;

        fee += auction.fees;

        if auction.is_alien {
            let total_fees = (auction.total_amount * seller_fee) / 1000;
            fee += total_fees;
            owner_earnings += auction.total_amount - total_fees;
        }

        // Emit AuctionEnded event (replace with Solana event)
        msg!("Auction ended for listing: {}", listing_id);

        // Remove the listing from active auctions and add to past auctions
        if
            let Some(index) = auction_state.active_auctions[&auction.owner]
                .iter()
                .position(|x| *x == listing_id)
        {
            auction_state.active_auctions.get_mut(&auction.owner).unwrap().remove(index);
            auction_state.past_auctions.get_mut(&auction.owner).unwrap().push(listing_id.clone());
        }

        // Generate Metadata for minting
        let metadata = generate_metadata(
            listing_id.clone(),
            auction.highest_bid,
            auction.bids.get(&auction.highest_bidder).unwrap().time,
            auction.owner,
            ctx.accounts.system_program.key()
        );

        // Try minting
        if
            let Err(_) = mint_nft(
                auction.highest_bidder,
                listing_id.clone(),
                metadata,
                auction.owner,
                auction.highest_bid,
                hook
            )
        {
            // Minting failed, revert with custom error
            return Err(ErrorCode::MintingFailed.into());
        }

        // Transfer funds to the owner and fee recipient
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.owner.key(),
                &auction.owner,
                owner_earnings
            ),
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.fee_recipient.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ]
        )?;

        invoke(
            &system_instruction::transfer(
                &ctx.accounts.owner.key(),
                &ctx.accounts.fee_recipient.key(),
                fee
            ),
            &[
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.fee_recipient.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ]
        )?;

        Ok(())
    }

    pub fn generate_metadata(
        listing_id: &str,
        amount: u64,
        time: i64,
        seller_address: Pubkey,
        payment_contract_address: Pubkey
    ) -> String {
        // Perform validation checks
        require!(seller_address != Pubkey::default(), ErrorCode::InvalidSellerAddress);
        require!(
            payment_contract_address != Pubkey::default(),
            ErrorCode::InvalidPaymentContractAddress
        );

        // Convert amount and time to string
        let amount_str = uint_to_string(amount);
        let time_str = uint_to_string(time as u64);

        // Construct the metadata string using format!
        let metadata = format!(
            "listing_id:{}, amount:{}, time:{}, seller:{}, minter:{}",
            listing_id,
            amount_str,
            time_str,
            seller_address,
            payment_contract_address
        );
        metadata
    }

    pub fn uint_to_string(value: u64) -> String {
        // Convert the unsigned integer to a string using Rust's built-in method
        value.to_string()
    }

    pub fn get_highest_bidder(ctx: Context<GetHighestBidder>, listing_id: String) -> Pubkey {
        let auction_state = &ctx.accounts.auction_state;
        auction_state.highest_bidder
    }

    pub fn get_auction_end_time(ctx: Context<GetAuctionEndTime>, listing_id: String) -> i64 {
        let auction_state = &ctx.accounts.auction_state;
        auction_state.end_time
    }

    pub fn has_auction_ended(ctx: Context<HasAuctionEnded>, listing_id: String) -> bool {
        let auction_state = &ctx.accounts.auction_state;
        auction_state.ended
    }

    pub fn get_active_auctions_of(ctx: Context<GetActiveAuctionsOf>, owner: Pubkey) -> Vec<String> {
        let auction_data = &ctx.accounts.auction_data;

        auction_data.active_auctions
            .get(&owner)
            .unwrap_or(&vec![])
            .clone()
    }

    // Function to get past auctions for an owner
    pub fn get_past_auctions_of(ctx: Context<GetPastAuctionsOf>, owner: Pubkey) -> Vec<String> {
        let auction_data = &ctx.accounts.auction_data;

        auction_data.past_auctions
            .get(&owner)
            .unwrap_or(&vec![])
            .clone()
    }

    // Function to get pending withdrawals for an owner
    pub fn get_pending_withdrawals(ctx: Context<GetPendingWithdrawals>, address: Pubkey) -> u64 {
        let auction_data = &ctx.accounts.auction_data;

        *auction_data.pending_withdrawals.get(&address).unwrap_or(&0)
    }

    pub fn get_bid_amount(ctx: Context<GetBidAmount>, listing_id: String, bidder: Pubkey) -> u64 {
        let auction = &ctx.accounts.auction;

        // Check if the bidder exists in the bids mapping
        if let Some(bid) = auction.bids.get(&bidder) {
            bid.amount
        } else {
            0 // Return 0 if no bid exists for the bidder
        }
    }

    pub fn get_auction_details(
        ctx: Context<GetAuctionDetails>,
        listing_id: String
    ) -> AuctionDetailsResponse {
        let auction = &ctx.accounts.auction;

        // Create and return an AuctionDetailsResponse struct
        AuctionDetailsResponse {
            listing_id: auction.listing_id.clone(),
            highest_bid: auction.highest_bid,
            highest_bidder: auction.highest_bidder,
            minimum_bid: auction.minimum_bid,
            ended: auction.ended,
            owner: auction.owner,
            end_time: auction.end_time,
            bidders: auction.bidders.clone(),
            num_bidders: auction.bidders.len() as u64,
        }
    }

    pub fn get_pending_withdrawal_amount(
        ctx: Context<GetPendingWithdrawalAmount>,
        owner: Pubkey
    ) -> u64 {
        let auction_details = &ctx.accounts.auction_details;

        // Retrieve the pending withdrawal amount for the owner
        *auction_details.pending_withdrawals.get(&owner).unwrap_or(&0)
    }

    pub fn get_highest_bid_and_end_time(
        ctx: Context<GetHighestBidAndEndTime>,
        listing_id: String
    ) -> Result<(Pubkey, u64, i64, u64)> {
        let auction = &ctx.accounts.auction_details;

        // Get the current time
        let current_time = Clock::get()?.unix_timestamp;

        // Calculate the remaining time
        let remaining_time = if current_time < auction.end_time {
            auction.end_time - current_time
        } else {
            0
        };

        Ok((auction.highest_bidder, auction.highest_bid, auction.end_time, remaining_time))
    }

    pub fn get_winner(ctx: Context<GetWinner>, listing_id: String) -> Result<Pubkey> {
        let auction = &ctx.accounts.auction_details;

        // Check if the auction has ended
        require!(auction.ended, ErrorCode::AuctionNotEnded); // Custom error for auction not ended

        Ok(auction.highest_bidder)
    }
}

#[account]
pub struct AuctionDetails {
    pub listing_id: String,
    pub highest_bid: u64,
    pub highest_bidder: Pubkey,
    pub bids: std::collections::HashMap<Pubkey, Bid>,
    pub minimum_bid: u64,
    pub end_time: i64,
    pub fees: u64,
    pub ended: bool,
    pub paused: bool,
    pub is_alien: bool,
    pub total_amount: u64,
    pub owner: Pubkey,
    pub bidders: Vec<Pubkey>, // Store bidders' public keys
    pub active_auctions: HashMap<Pubkey, Vec<String>>,
    pub past_auctions: HashMap<Pubkey, Vec<String>>,
    pub pending_withdrawals: HashMap<Pubkey, u64>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Bid {
    pub amount: u64,
    pub time: i64,
}

#[state]
pub struct NftComAuction {
    pub auctions: HashMap<String, AuctionDetails>,
    pub active_auctions: HashMap<Pubkey, Vec<String>>,
    pub past_auctions: HashMap<Pubkey, Vec<String>>,
    pub pending_withdrawals: HashMap<Pubkey, u64>,
    pub fee_recipient: Pubkey,
    pub active_bids: HashMap<Pubkey, Vec<String>>,
    pub buyer_fee: u64,
    pub seller_fee: u64,
}

#[event]
pub struct AuctionEnded {
    pub listing_id: String,
    pub winner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct AuctionInitialized {
    pub listing_id: String,
    pub minimum: u64,
    pub end_time: i64,
}

#[event]
pub struct BidPlaced {
    pub listing_id: String,
    pub sender: Pubkey,
    pub value: u64,
}

#[derive(Accounts)]
pub struct ChangeFeeRecipient<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct ChangeNFTContract<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct SetFees<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct EmergencyPauseAuction<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct InitializeAuction<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct PlaceBid<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    pub owner: Signer<'info>,
    pub bid_amount: u64, // Amount of the bid
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, NftComAuction>,
    #[account(mut)]
    pub bidder: Signer<'info>,
    #[account(mut)]
    pub to: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct GetUserBid<'info> {
    pub auction_state: Account<'info, NftComAuction>,
}

#[derive(Accounts)]
pub struct GetAllBidsOfUser<'info> {
    pub auction_state: Account<'info, NftComAuction>,
}

#[derive(Accounts)]
pub struct GetLatestBids<'info> {
    pub auction_state: Account<'info, NftComAuction>,
}

#[derive(Accounts)]
pub struct EndAuction<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, AuctionState>,
    pub owner: Signer<'info>,
    #[account(mut)]
    pub fee_recipient: AccountInfo<'info>,
    #[account(mut)]
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct GetHighestBidder<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, AuctionState>,
}

#[derive(Accounts)]
pub struct GetAuctionEndTime<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, AuctionState>,
}

#[derive(Accounts)]
pub struct HasAuctionEnded<'info> {
    #[account(mut)]
    pub auction_state: Account<'info, AuctionState>,
}

#[derive(Accounts)]
pub struct GetActiveAuctionsOf<'info> {
    #[account(mut)]
    pub auction_data: Account<'info, AuctionData>, // The account holding auction data
}

#[derive(Accounts)]
pub struct GetPastAuctionsOf<'info> {
    #[account(mut)]
    pub auction_data: Account<'info, AuctionData>, // The account holding auction data
}

#[derive(Accounts)]
pub struct GetPendingWithdrawals<'info> {
    #[account(mut)]
    pub auction_data: Account<'info, AuctionData>, // The account holding auction data
}

#[derive(Accounts)]
pub struct GetBidAmount<'info> {
    #[account(mut)]
    pub auction: Account<'info, Auction>, // The auction account holding bid data
}

#[derive(Accounts)]
pub struct GetAuctionDetails<'info> {
    #[account(mut)]
    pub auction: Account<'info, AuctionDetails>, // The auction account holding details
}

#[derive(Accounts)]
pub struct GetPendingWithdrawalAmount<'info> {
    #[account(mut)]
    pub auction_details: Account<'info, AuctionDetails>,
}

#[derive(Accounts)]
pub struct GetHighestBidAndEndTime<'info> {
    #[account(mut)]
    pub auction_details: Account<'info, AuctionDetails>,
}

#[derive(Accounts)]
pub struct GetWinner<'info> {
    #[account(mut)]
    pub auction_details: Account<'info, AuctionDetails>,
}

#[error]
pub enum ErrorCode {
    #[msg("Invalid listing ID.")]
    InvalidListingId,
    #[msg("Auction has not ended yet.")]
    AuctionNotEnded,
    #[msg("Invalid seller address.")]
    InvalidSellerAddress,
    #[msg("Invalid payment contract address.")]
    InvalidPaymentContractAddress,
}

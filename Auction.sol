// SPDX-License-Identifier: MIT
pragma solidity ^0.8.9;

import "@openzeppelin/contracts/security/ReentrancyGuard.sol";
import "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/Strings.sol";

interface IERC721Mintable is IERC721 {
    function mint(
        address to,
        string memory _listing_id,
        string memory _order_id,
        string memory metadata,
        address seller,
        uint256 amount,
        address hook
    ) external;
}

//0x0BeeabCE39716AdBEEdfD5c290826863Dc367Aeb
contract NftComAuction is Ownable, ReentrancyGuard {
    struct Bid {
        uint256 amount;
        uint256 time;
    }

    struct AuctionDetails {
        string listingId;
        uint256 highestBid;
        address highestBidder;
        uint256 minimumBid;
        uint256 endTime;
        uint256 fees;
        bool ended;
        bool paused;
        bool isAlien;
        uint256 totalAmount;
        address payable owner;
        mapping(address => Bid) bids;
        address[] bidders;
    }
    // Define the global constants for sniping protection
    uint256 public constant snipingTimeWindow = 300; // 300 seconds
    uint256 public constant timeExtension = 300; // 300 seconds

    mapping(string => AuctionDetails) public auctions;
    mapping(address => string[]) public activeAuctions;
    mapping(address => string[]) public pastAuctions;
    mapping(address => uint256) public pendingWithdrawals;

    address payable private feeRecipient;

    mapping(address => string[]) public activeBids;

    uint256 public buyerFee = 15;
    uint256 public sellerFee = 15;

    event AuctionEnded(string listingId, address winner, uint256 amount);
    event AuctionInitialized(
        string listingId,
        uint256 _minimum,
        uint256 _endTime
    );
    event BidPlaced(string _listingId, address sender, uint256 value);

    IERC721Mintable private nftcom;

    constructor(address payable _feeRecipient, IERC721Mintable _nftcom) {
        feeRecipient = _feeRecipient;
        nftcom = _nftcom;
    }

    function changeFeeRecipient(
        address payable _feeRecipient
    ) public onlyOwner {
        feeRecipient = _feeRecipient;
    }

    function changeNFTContract(IERC721Mintable _address) public onlyOwner {
        nftcom = _address;
    }

    function setFees(uint256 _buyerFee, uint256 _sellerFee) public onlyOwner {
        buyerFee = _buyerFee;
        sellerFee = _sellerFee;
    }

    function emergencyPauseAuction(
        string memory _listingId,
        bool _status
    ) public onlyOwner {
        auctions[_listingId].paused = _status;
    }

    function initializeAuction(
        string memory _listingId,
        uint256 _minimum,
        uint256 _endTime,
        address payable _owner,
        address bidder
    ) public payable {
        if (address(0) == bidder) {
            bidder = msg.sender;
        }

        if (auctions[_listingId].endTime == 0) {
            require(_minimum > 0, "Minimum bid should be greater than zero.");
            require(
                _endTime > block.timestamp,
                "End time should be in the future."
            );
            require(
                _owner != address(0),
                "Owner address can't be the zero address."
            );
            AuctionDetails storage auction = auctions[_listingId];
            auction.listingId = _listingId;
            auction.minimumBid = _minimum;
            auction.endTime = _endTime;
            auction.owner = _owner;
            auction.fees = 0;
            auction.paused = false;
            auction.isAlien = false;
            auction.bidders = new address[](0);
            activeAuctions[_owner].push(_listingId);
            placeBid(_listingId, bidder);
            emit AuctionInitialized(_listingId, _minimum, _endTime);
        } else {
            placeBid(_listingId, bidder);
        }
    }

    function placeBid(string memory _listingId, address bidder) public payable {
        AuctionDetails storage auction = auctions[_listingId];

        require(bidder != address(0), "Invalid address");
        require(bidder != auction.owner, "Bidder cannot be auction owner");
        require(
            msg.sender != auction.owner,
            "Bidder should not be auction owner"
        );

        if (address(0) == bidder) {
            bidder = msg.sender;
        }

        uint256 fee = (msg.value * buyerFee) / 1000;
        uint256 bidAmount = msg.value - fee;

        require(auction.endTime != 0, "Auction does not exist.");
        require(auction.ended == false, "Auction has already ended.");
        require(auction.paused == false, "Auction has been paused");
        require(block.timestamp <= auction.endTime, "Auction ended");

        // Check for sniping protection
        if (block.timestamp >= auction.endTime - snipingTimeWindow) {
            auction.endTime += timeExtension;
        }
        auction.totalAmount += bidAmount;
        uint256 newBid = auction.bids[bidder].amount + bidAmount;

        require(
            newBid > (auction.highestBid * 103) / 100,
            "Bid must be at least 3% higher than the current highest bid."
        );

        if (bidder == auction.highestBidder) {
            // The same bidder is increasing their bid
            auction.bids[bidder].amount = newBid;
            auction.highestBid = newBid;
            auction.bids[bidder].time = block.timestamp;
        } else {
            // This is a new bidder
            require(
                bidAmount >= auction.minimumBid,
                "Bid amount should be higher than the minimum"
            );

            auction.highestBid = newBid;
            auction.highestBidder = bidder;
            auction.bids[bidder].amount = newBid;
            auction.bids[bidder].time = block.timestamp;
            auction.bidders.push(bidder);

            bool isBidding = false;
            for (uint i = 0; i < activeBids[bidder].length; i++) {
                if (
                    keccak256(abi.encodePacked(activeBids[bidder][i])) ==
                    keccak256(abi.encodePacked(_listingId))
                ) {
                    isBidding = true;
                    break;
                }
            }
            if (!isBidding) {
                activeBids[bidder].push(_listingId);
            }
        }

        auction.fees += fee;

        // pendingWithdrawals[feeRecipient] += fee;
        // pendingWithdrawals[auction.owner] += ownerProceeds;

        emit BidPlaced(_listingId, bidder, bidAmount);
    }

    function withdraw(
        string memory _listingId,
        address _to
    ) public payable nonReentrant {
        require(
            auctions[_listingId].isAlien == false,
            "Withdrawal is not possible for alien auctions. You can only increease your bid"
        );
        require(
            msg.sender != auctions[_listingId].highestBidder,
            "Highest bidder cannot withdraw"
        );
        uint256 refundAmount = auctions[_listingId].bids[msg.sender].amount;
        require(refundAmount > 0, "No funds to withdraw.");
        if (address(0) == _to) {
            _to = msg.sender;
        }
        auctions[_listingId].bids[_to].amount = 0;
        payable(_to).transfer(refundAmount);
    }

    // other methods
    function getUserBid(
        string memory _listingId,
        address user
    ) public view returns (address, uint256, uint256) {
        AuctionDetails storage auction = auctions[_listingId];
        if (auction.bids[user].amount != 0) {
            return (user, auction.bids[user].amount, auction.bids[user].time);
        }
        return (address(0), 0, 0);
    }

    function getAllBidsOfUser(
        address bidder
    )
        public
        view
        returns (
            string[] memory listingIds,
            uint256[] memory bidAmounts,
            uint256[] memory bidTimes
        )
    {
        string[] memory activeBidsForUser = activeBids[bidder];
        uint256[] memory amounts = new uint256[](activeBidsForUser.length);
        uint256[] memory times = new uint256[](activeBidsForUser.length);

        for (uint i = 0; i < activeBidsForUser.length; i++) {
            string memory listingId = activeBidsForUser[i];
            amounts[i] = auctions[listingId].bids[bidder].amount;
            times[i] = auctions[listingId].bids[bidder].time;
        }

        return (activeBidsForUser, amounts, times);
    }

    function getLatestBids(
        string memory _listingId,
        uint256 n
    )
        public
        view
        returns (address[] memory, uint256[] memory, uint256[] memory)
    {
        AuctionDetails storage auction = auctions[_listingId];
        address[] memory latestBidders = new address[](n);
        uint256[] memory latestBidAmounts = new uint256[](n);
        uint256[] memory latestBidTimes = new uint256[](n);

        uint256 length = auction.bidders.length;
        if (n > length) {
            n = length;
        }

        for (uint256 i = 0; i < n; i++) {
            address bidder = auction.bidders[length - 1 - i];
            latestBidders[i] = bidder;
            latestBidAmounts[i] = auction.bids[bidder].amount;
            latestBidTimes[i] = auction.bids[bidder].time;
        }
        return (latestBidders, latestBidAmounts, latestBidTimes);
    }

    function endAuction(
        string memory _listingId,
        address _hook
    ) public nonReentrant {
        AuctionDetails storage auction = auctions[_listingId];
        require(block.timestamp >= auction.endTime, "Auction not yet ended.");
        require(!auction.ended, "Auction end has already been called.");
        require(auction.highestBid > 0, "Nothing to Withdraw");

        auction.ended = true;
        uint256 fee = ((auction.highestBid * sellerFee) / 1000);
        uint256 ownerEarnings = auction.highestBid - fee;
        fee += auction.fees;
        if (auction.isAlien) {
            uint256 totalFees = ((auction.totalAmount * sellerFee) / 1000);
            fee += totalFees;
            ownerEarnings += auction.totalAmount - totalFees;
        }

        // pendingWithdrawals[feeRecipient] += fee ;

        emit AuctionEnded(_listingId, auction.highestBidder, ownerEarnings);

        for (uint i = 0; i < activeAuctions[auction.owner].length; i++) {
            if (
                keccak256(abi.encodePacked(activeAuctions[auction.owner][i])) ==
                keccak256(abi.encodePacked(_listingId))
            ) {
                activeAuctions[auction.owner][i] = activeAuctions[
                    auction.owner
                ][activeAuctions[auction.owner].length - 1];
                activeAuctions[auction.owner].pop();
                pastAuctions[auction.owner].push(_listingId);
                break;
            }
        }

        string memory metadata = generateMetadata(
            _listingId,
            auction.highestBid,
            auction.bids[auction.highestBidder].time,
            auction.owner,
            address(this)
        );

        try
            nftcom.mint(
                auction.highestBidder,
                _listingId,
                _listingId,
                metadata,
                auction.owner,
                auction.highestBid,
                _hook
            )
        {
            (bool success, ) = auction.owner.call{value: ownerEarnings}("");
            require(success, "Owner Funds Transfer failed");
            (bool feeSent, ) = feeRecipient.call{value: fee}("");
            require(feeSent, "Fee Transfer failed");
        } catch {
            // Minting failed, handle the error (revert with a custom error message, emit an event, etc.)
            revert("NFT minting failed");
        }
    }

    function generateMetadata(
        string memory listing_id,
        uint256 amount,
        uint256 time,
        address sellerAddress,
        address paymentContractAddress
    ) internal pure returns (string memory) {
        // You might want to add validation checks here
        require(
            sellerAddress != address(0),
            "Seller address cannot be zero address"
        );
        require(
            paymentContractAddress != address(0),
            "Payment Contract address cannot be zero address"
        );

        // Construct the metadata string
        return
            string(
                abi.encodePacked(
                    "listing_id:",
                    listing_id,
                    ", amount:",
                    Strings.toString(amount),
                    ", time:",
                    Strings.toString(time),
                    ", seller:",
                    _address2str(sellerAddress),
                    ", minter:",
                    _address2str(paymentContractAddress)
                )
            );
    }

    // Converts a uint to a string
    function _uint2str(
        uint256 _i
    ) internal pure returns (string memory _uintAsString) {
        if (_i == 0) {
            return "0";
        }
        uint256 j = _i;
        uint256 length;
        while (j != 0) {
            length++;
            j /= 10;
        }
        bytes memory bstr = new bytes(length);
        uint256 k = length - 1;
        while (_i != 0) {
            bstr[k--] = bytes1(uint8(48 + uint8(_i % 10)));
            _i /= 10;
        }
        return string(bstr);
    }

    function _address2str(address _addr) internal pure returns (string memory) {
        bytes32 value = bytes32(uint256(uint160(_addr)));
        bytes memory alphabet = "0123456789abcdef";

        bytes memory str = new bytes(42);
        str[0] = "0";
        str[1] = "x";
        for (uint256 i = 0; i < 20; i++) {
            str[2 + i * 2] = alphabet[uint8(value[i + 12] >> 4)];
            str[3 + i * 2] = alphabet[uint8(value[i + 12] & 0x0f)];
        }
        return string(str);
    }

    function getHighestBidder(
        string memory _listingId
    ) public view returns (address) {
        return auctions[_listingId].highestBidder;
    }

    function getAuctionEndTime(
        string memory _listingId
    ) public view returns (uint256) {
        return auctions[_listingId].endTime;
    }

    function hasAuctionEnded(
        string memory _listingId
    ) public view returns (bool) {
        return auctions[_listingId].ended;
    }

    function getPendingWithdrawals(
        address _address
    ) public view returns (uint256) {
        return pendingWithdrawals[_address];
    }

    function getActiveAuctionsOf(
        address _owner
    ) public view returns (string[] memory) {
        return activeAuctions[_owner];
    }

    function getPastAuctionsOf(
        address _owner
    ) public view returns (string[] memory) {
        return pastAuctions[_owner];
    }

    function getBidAmount(
        string memory _listingId,
        address _bidder
    ) public view returns (uint256) {
        return auctions[_listingId].bids[_bidder].amount;
    }

    function getAuctionDetails(
        string memory _listingId
    )
        public
        view
        returns (
            string memory,
            uint256,
            address,
            uint256,
            bool,
            address,
            uint256,
            address[] memory,
            uint256
        )
    {
        AuctionDetails storage auction = auctions[_listingId];
        return (
            auction.listingId,
            auction.highestBid,
            auction.highestBidder,
            auction.minimumBid,
            auction.ended,
            auction.owner,
            auction.endTime,
            auction.bidders,
            auction.bidders.length
        );
    }

    function getPendingWithdrawalAmount(
        address _owner
    ) public view returns (uint256) {
        return pendingWithdrawals[_owner];
    }

    function getHighestBidAndEndTime(
        string memory _listingId
    ) public view returns (address, uint256, uint256, uint256) {
        AuctionDetails storage auction = auctions[_listingId];
        uint256 currentTime = block.timestamp;
        uint256 remainingTime = 0;

        if (currentTime < auction.endTime) {
            remainingTime = auction.endTime - currentTime;
        }

        return (
            auction.highestBidder,
            auction.highestBid,
            auction.endTime,
            remainingTime
        );
    }

    function getWinner(string memory _listingId) public view returns (address) {
        require(auctions[_listingId].ended, "Auction has not ended yet."); // Added line
        return auctions[_listingId].highestBidder;
    }
}

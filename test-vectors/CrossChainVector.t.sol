// SPDX-License-Identifier: MIT
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {QubiHTLC} from "../src/QubiHTLC.sol";
import {MockUSDT} from "../src/test-tokens/MockTokens.sol";

contract CrossChainVectorTest is Test {
    bytes32 constant S = 0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20;
    bytes32 constant H = 0xae216c2ef5247a3782c135efa279a3e4cdc61094270f5d2be58c6204b7a612c9;

    function test_vector() public {
        assertEq(sha256(abi.encodePacked(S)), H);
        MockUSDT usdt = new MockUSDT("USDT", "USDT", 6);
        QubiHTLC htlc = new QubiHTLC(address(0xFEE0), 50, address(usdt), 1e12, 1e13);
        address buyer = address(0xB0B);
        address seller = address(0x5E11);
        usdt.mint(buyer, 1000e6);
        vm.startPrank(buyer);
        usdt.approve(address(htlc), type(uint256).max);
        bytes32 id = htlc.lock(bytes32(uint256(1)), seller, address(usdt), 1000e6, H, uint64(block.timestamp + 1 hours));
        vm.stopPrank();
        vm.prank(address(0xBAD));
        htlc.claim(id, S);
        assertEq(usdt.balanceOf(seller), 995e6);
        emit log_named_bytes32("sha256(abi.encodePacked(S))", sha256(abi.encodePacked(S)));
    }
}
